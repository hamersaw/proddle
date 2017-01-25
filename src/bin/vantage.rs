extern crate env_logger;
extern crate capnp;
extern crate capnp_rpc;
#[macro_use]
extern crate clap;
extern crate futures;
#[macro_use]
extern crate log;
extern crate proddle;
extern crate threadpool;
extern crate time;
extern crate tokio_core;

use capnp::capability::Promise;
use capnp_rpc::RpcSystem;
use capnp_rpc::twoparty::VatNetwork;
use capnp_rpc::rpc_twoparty_capnp::Side;
use clap::App;
use futures::Future;
use proddle::{Error, Measurement, Operation};
use proddle::proddle_capnp::proddle::Client;
use threadpool::ThreadPool;
use tokio_core::io::Io;
use tokio_core::net::TcpStream;
use tokio_core::reactor::Core;

use std::cmp::{Ordering, PartialOrd};
use std::collections::{BinaryHeap, HashMap};
use std::collections::hash_map::DefaultHasher;
use std::fs::File;
use std::hash::{Hash, Hasher};
use std::io::Write;
use std::net::SocketAddr;
use std::process::Command;
use std::str::FromStr;
use std::sync::{Arc, RwLock};

pub fn main() {
    env_logger::init().unwrap();
    let yaml = load_yaml!("vantage_args.yaml");
    let matches = App::from_yaml(yaml).get_matches();

    //initialize vantage parameters
    info!("parsing command line arguments");
    let hostname = matches.value_of("HOSTNAME").unwrap().to_owned();
    let ip_address = matches.value_of("IP_ADDRESS").unwrap().to_owned();
    let measurements_directory = matches.value_of("MEASUREMENTS_DIRECTORY").unwrap().to_owned();
    let bucket_count = match matches.value_of("BUCKET_COUNT").unwrap().parse::<u64>() {
        Ok(bucket_count) => bucket_count,
        Err(e) => panic!("failed to parse bucket_count as u64: {}", e),
    };

    let thread_count = match matches.value_of("THREAD_COUNT").unwrap().parse::<usize>() {
        Ok(thread_count) => thread_count,
        Err(e) => panic!("failed to parse thread_count as usize: {}", e),
    };

    let server_address = &format!("{}:{}", matches.value_of("SERVER_IP_ADDRESS").unwrap(), matches.value_of("SERVER_PORT").unwrap());
    let server_poll_interval_seconds = match matches.value_of("SERVER_POLL_INTERVAL_SECONDS").unwrap().parse::<u64>() {
        Ok(server_poll_interval_seconds) => server_poll_interval_seconds,
        Err(e) => panic!("failed to parse server_poll_interval_seconds as u64: {}", e),
    };

    let result_batch_size = match matches.value_of("RESULT_BATCH_SIZE").unwrap().parse::<usize>() {
        Ok(result_batch_size) => result_batch_size,
        Err(e) => panic!("failed to parse result_batch_size as usize: {}", e),
    };

    let include_tags = match matches.values_of("INCLUDE_TAGS") {
        Some(include_tags) => include_tags.collect(),
        None => Vec::new(),
    };

    let exclude_tags = match matches.values_of("EXCLUDE_TAGS") {
        Some(exclude_tags) => exclude_tags.collect(),
        None => Vec::new(),
    };

    info!("initializing vantage data structures");

    //initialize vantage data structures
    let measurements: Arc<RwLock<HashMap<String, Measurement>>> = Arc::new(RwLock::new(HashMap::new()));
    let operations: Arc<RwLock<HashMap<u64, BinaryHeap<OperationJob>>>> = Arc::new(RwLock::new(HashMap::new()));
    let operation_bucket_hashes: Arc<RwLock<HashMap<u64, u64>>> = Arc::new(RwLock::new(HashMap::new()));

    //populate operations with buckets
    {
        let mut operations = operations.write().unwrap();
        let mut operation_bucket_hashes = operation_bucket_hashes.write().unwrap();

        let mut counter = 0;
        let delta = u64::max_value() / bucket_count;
        for _ in 0..bucket_count {
            operations.insert(counter, BinaryHeap::new());
            operation_bucket_hashes.insert(counter, 0);
            counter += delta;
        }
    }

    //create result channels
    let (tx, rx) = std::sync::mpsc::channel();

    info!("service started");

    //start recv result channel
    let thread_server_address = server_address.clone();
    std::thread::spawn(move || {
        let mut result_buffer: Vec<String> = Vec::new();
        let mut failure_retry_time = 0;

        loop {
            match rx.recv() {
                Ok(result) => result_buffer.push(result),
                Err(e) => panic!("failed to retrieve result from result channel: {}", e),
            };

            if result_buffer.len() >= result_batch_size && time::now_utc().to_timespec().sec > failure_retry_time {
                info!("sending {} results to server", result_buffer.len());
                if let Err(e) = send_results(&mut result_buffer, &thread_server_address) {
                    error!("failed to send results: {}", e);
                    failure_retry_time = time::now_utc().to_timespec().sec + (60 * 10);
                };
            }
        }
    });

    //start thread for scheduling operations
    let thread_operations = operations.clone();
    let thread_measurements_directory = measurements_directory.to_owned();
    std::thread::spawn(move || {
        let thread_pool = ThreadPool::new(thread_count);

        loop {
            let now = time::now_utc().to_timespec().sec;

            //iterate over buckets of operation jobs
            {
                let mut operations = thread_operations.write().unwrap();
                for (_, operation_jobs) in operations.iter_mut() {
                    loop {
                        let execution_time = match operation_jobs.peek() {
                            Some(operation_job) => operation_job.execution_time,
                            None => break,
                        };

                        //if the next execution time is earlier then the current time then execute
                        if execution_time < now {
                            let mut operation_job = operation_jobs.pop().unwrap();
                            let pool_operation_job = operation_job.clone();
                            operation_job.execution_time += operation_job.operation.interval as i64;
                            operation_jobs.push(operation_job);

                            //add job to thread pool
                            let pool_tx = tx.clone();
                            let pool_hostname = hostname.clone();
                            let pool_ip_address = ip_address.clone();
                            let pool_measurements_directory = thread_measurements_directory.clone();
                            thread_pool.execute(move || {
                                //execute operation and store results in json string
                                let mut result = String::from_str("{").unwrap();
                                result.push_str(&format!("\"Timestamp\":{}", time::now_utc().to_timespec().sec));
                                result.push_str(&format!(",\"Hostname\":\"{}\"", pool_hostname));
                                result.push_str(&format!(",\"IpAddress\":\"{}\"", pool_ip_address));
                                result.push_str(&format!(",\"Measurement\":\"{}\"", pool_operation_job.operation.measurement));
                                result.push_str(&format!(",\"Domain\":\"{}\"", pool_operation_job.operation.domain));

                                match Command::new("python")
                                            .arg(format!("{}/{}", pool_measurements_directory, pool_operation_job.operation.measurement))
                                            .arg(pool_operation_job.operation.domain)
                                            .output() {
                                    Ok(output) => {
                                        result.push_str(&format!(",\"Error\":false,\"Result\":{}", String::from_utf8_lossy(&output.stdout).into_owned()));
                                    },
                                    Err(e) => result.push_str(&format!(",\"Error\":true,\"ErrorMessage\":\"{}\"", e)),
                                };

                                result.push_str("}");

                                //send result string over result channel
                                if let Err(e) = pool_tx.send(result) {
                                    panic!("failed to send result over result channel: {}", e);
                                }
                            });
                        } else {
                            break;
                        }
                    }
                }
            }

            std::thread::sleep(std::time::Duration::new(5, 0));
        }
    });

    //start loop to periodically request measurements and operations
    loop {
        info!("polling server");
        if let Err(e) = poll_server(measurements.clone(), &measurements_directory, operations.clone(), operation_bucket_hashes.clone(), &include_tags, &exclude_tags, server_address) {
            error!("failed to poll server: {}", e);
        }

        std::thread::sleep(std::time::Duration::new(server_poll_interval_seconds, 0))
    }
}

fn send_results(result_buffer: &mut Vec<String>, server_address: &str) -> Result<(), Error> {
    //open stream
    let mut core = try!(Core::new());
    let handle = core.handle();
                        
    let socket_addr = try!(SocketAddr::from_str(server_address));
    let stream = try!(core.run(TcpStream::connect(&socket_addr, &handle)));

    try!(stream.set_nodelay(true));
    let (reader, writer) = stream.split();

    let network = Box::new(VatNetwork::new(reader, writer, Side::Client, Default::default()));
    let mut rpc_system = RpcSystem::new(network, None);
    let proddle: Client = rpc_system.bootstrap(Side::Server);
    handle.spawn(rpc_system.map_err(|e| error!("{:?}", e)));

    //initialize request
    let mut request = proddle.send_results_request();
    {
        let mut request_results = request.get().init_results(result_buffer.len() as u32);
        for (i, result) in result_buffer.iter().enumerate() {
            let mut request_result = request_results.borrow().get(i as u32);
            request_result.set_json_string(result);
        }
    }

    //send request and read response
    try!(
        core.run(request.send().promise.and_then(|_| {
            Promise::ok(())
        }))
    );

    //clear result buffer
    result_buffer.clear();
    Ok(())
}

fn poll_server(
        measurements: Arc<RwLock<HashMap<String, Measurement>>>,
        measurements_directory: &str,
        operations: Arc<RwLock<HashMap<u64, BinaryHeap<OperationJob>>>>,
        operation_bucket_hashes: Arc<RwLock<HashMap<u64, u64>>>,
        include_tags: &Vec<&str>,
        exclude_tags: &Vec<&str>,
        server_address: &str) -> Result<(), Error> {
    //open stream
    let mut core = try!(Core::new());
    let handle = core.handle();
                        
    let socket_addr = try!(SocketAddr::from_str(server_address));
    let stream = try!(core.run(TcpStream::connect(&socket_addr, &handle)));

    try!(stream.set_nodelay(true));
    let (reader, writer) = stream.split();

    let network = Box::new(VatNetwork::new(reader, writer, Side::Client, Default::default()));
    let mut rpc_system = RpcSystem::new(network, None);
    let proddle: Client = rpc_system.bootstrap(Side::Server);
    handle.spawn(rpc_system.map_err(|e| error!("{:?}", e)));

    //populate get measurements request
    let mut request = proddle.get_measurements_request();
    {
        let measurements = measurements.read().unwrap();
        let mut request_measurements = request.get().init_measurements(measurements.len() as u32);
        for (i, measurement) in measurements.values().enumerate() {
            let mut request_measurement = request_measurements.borrow().get(i as u32);

            if let Some(timestamp) = measurement.timestamp {
                request_measurement.set_timestamp(timestamp);
            }

            request_measurement.set_name(&measurement.name);
            request_measurement.set_version(measurement.version);
        }
    }

    //send get measurements request
    let mut measurements_added = 0;
    let response = try!(core.run(request.send().promise));
    {
        let result_measurements = try!(try!(response.get()).get_measurements());
        
        let mut measurements = measurements.write().unwrap();
        for result_measurement in result_measurements.iter() {
            let measurement = try!(Measurement::from_capnproto(&result_measurement));
            if measurement.version == 0 {
                //delete file
                try!(std::fs::remove_file(format!("{}/{}", measurements_directory, measurement.name)));

                //remove from measurements data structures
                measurements.remove(&measurement.name);
            } else {
                //create file
                let mut file = try!(File::create(format!("{}/{}", measurements_directory, measurement.name)));

                let content = measurement.content.clone().unwrap().into_bytes();
                try!(file.write_all(&content));
                try!(file.flush());

                //add to measurements data structure
                measurements.insert(measurement.name.to_owned(), measurement);
                measurements_added += 1;
            }
        }
    }

    if measurements_added > 0 {
        info!("added {} measurements", measurements_added);
    }

    //populate get operations request
    let mut request = proddle.get_operations_request();
    {
        let operation_bucket_hashes = operation_bucket_hashes.read().unwrap();
        let mut request_bucket_hashes = request.get().init_bucket_hashes(operation_bucket_hashes.len() as u32);
        for (i, (bucket_key, bucket_hash)) in operation_bucket_hashes.iter().enumerate() {
            let mut request_bucket_hash = request_bucket_hashes.borrow().get(i as u32);

            request_bucket_hash.set_bucket(*bucket_key);
            request_bucket_hash.set_hash(*bucket_hash);
        }
    }

    //send get operations request
    let mut operations_added = 0;
    let response = try!(core.run(request.send().promise));
    {
        let result_operation_buckets = try!(try!(response.get()).get_operation_buckets());

        let mut operations = operations.write().unwrap();
        let mut operation_bucket_hashes = operation_bucket_hashes.write().unwrap();
        for result_operation_bucket in result_operation_buckets.iter() {
            let mut binary_heap = BinaryHeap::new();
            let mut hasher = DefaultHasher::new();
            for result_operation in try!(result_operation_bucket.get_operations()).iter() {
                //add operation to binary heap
                let operation = try!(Operation::from_capnproto(&result_operation));
                operation.hash(&mut hasher);

                //validate tags
                if let Some(ref operation_tags) = operation.tags {
                    if !contains_tags(include_tags, &operation_tags) || contains_tags(exclude_tags, &operation_tags) {
                        continue;
                    }
                } else {
                    continue;
                }

                //add operation
                binary_heap.push(OperationJob::new(operation));
                operations_added += 1;
            }

            //insert new operations into operations map
            operations.insert(result_operation_bucket.get_bucket(), binary_heap);
            operation_bucket_hashes.insert(result_operation_bucket.get_bucket(), hasher.finish());
        }
    }

    if operations_added > 0 {
        info!("added {} operations", operations_added);
    }

    Ok(())
}

fn contains_tags(tags: &Vec<&str>, operation_tags: &Vec<String>) -> bool {
    for operation_tag in operation_tags {
        for tag in tags {
            if operation_tag.eq(*tag) {
                return true;
            }
        }
    }

    false
}

/*
 * OperationJob implementation
 */
#[derive(Clone)]
struct OperationJob  {
    execution_time: i64,
    operation: Operation,
}

impl OperationJob {
    fn new(operation: Operation) -> OperationJob {
        let now = time::now_utc().to_timespec().sec;

        OperationJob {
            execution_time: (now - (now % operation.interval as i64) + operation.interval as i64),
            operation: operation,
        }
    }
}

impl PartialEq for OperationJob {
    fn eq(&self, other: &OperationJob) -> bool {
        self.execution_time == other.execution_time
    }
}

impl Eq for OperationJob {}

impl Ord for OperationJob {
    fn cmp(&self, other: &OperationJob) -> Ordering {
        other.execution_time.cmp(&self.execution_time)
    }
}

impl PartialOrd for OperationJob {
    fn partial_cmp(&self, other: &OperationJob) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
