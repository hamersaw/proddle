#[macro_use(bson, doc)]
extern crate bson;
#[macro_use]
extern crate clap;
extern crate mongodb;
extern crate proddle;
extern crate rand;
extern crate rustc_serialize;
extern crate time;

use bson::Bson;
use clap::App;
use mongodb::ThreadedClient;
use mongodb::db::ThreadedDatabase;

use std::fs::File;
use std::io::Read;

fn main() {
    let yaml = load_yaml!("yogi_args.yaml");
    let matches = App::from_yaml(yaml).get_matches();

    //connect to mongodb
    let client = match proddle::get_mongodb_client("localhost", 27017) {
        Ok(client) => client,
        Err(e) => panic!("{}", e),
    };

    if let Some(matches) = matches.subcommand_matches("module") {
        if let Some(matches) = matches.subcommand_matches("add") {
            let file = matches.value_of("FILE").unwrap();
            let module_name = matches.value_of("MODULE_NAME").unwrap();
            let dependencies: Vec<Bson> = match matches.values_of("DEPENDENCY") {
                Some(dependencies) => dependencies.map(|x| Bson::String(x.to_owned())).collect(),
                None => Vec::new(),
            };

            let version = match proddle::find_module(client.clone(), module_name, None, true) {
                Ok(Some(document)) => {
                    match document.get("version") {
                        Some(&Bson::I32(document_version)) => document_version + 1,
                        _ => panic!("failed to parse version as i32"),
                    }
                },
                _ => 1,
            };

            //read file into compressed binary buffer
            let mut file = match File::open(file) {
                Ok(file) => file,
                Err(e) => panic!("failed to open file : {}", e),
            };

            let mut buffer = String::new();
            if let Err(e) = file.read_to_string(&mut buffer) {
                panic!("failed to read local file: {}", e);
            }

            //create module document
            let timestamp = time::now_utc().to_timespec().sec;
            let document = doc! { 
                "timestamp" => timestamp,
                "name" => module_name,
                "version" => version,
                "dependencies" => dependencies,
                "content" => buffer
            };

            //insert document
            if let Err(e) = client.db("proddle").collection("modules").insert_one(document, None) {
                panic!("failed to upload module document: {}", e);
            }
        } else if let Some(matches) = matches.subcommand_matches("delete") {
            unimplemented!();
        } else if let Some(matches) = matches.subcommand_matches("search") {
            let module_name = matches.value_of("MODULE_NAME").unwrap();

            match proddle::find_modules(client.clone(), Some(module_name), None, Some(1), true) {
                Ok(cursor) => {
                    for document in cursor {
                        let document = match document {
                            Ok(document) => document,
                            Err(e) => panic!("failed to retrieve document: {}", e),
                        };

                        println!("{:?}", document);
                    }
                },
                Err(e) => panic!("failed to find operations: {}", e),
            }
        }
    } else if let Some(matches) = matches.subcommand_matches("operation") {
        if let Some(matches) = matches.subcommand_matches("add") {
            let module_name = matches.value_of("MODULE_NAME").unwrap();
            let domain = matches.value_of("DOMAIN").unwrap();
            let interval = match matches.value_of("INTERVAL") {
                Some(interval) => {
                    match interval.parse::<i32>() {
                        Ok(interval) => interval,
                        Err(e) => panic!("failed to parse interval into integer: {}", e),
                    }
                },
                None => 14400,
            };

            //check if module exists
            match proddle::find_module(client.clone(), module_name, None, true) {
                Ok(Some(_)) => {},
                _ => panic!("module does not exist"),
            }

            //create opeation document
            let timestamp = time::now_utc().to_timespec().sec;
            let document = doc! {
                "timestamp" => timestamp,
                "module" => module_name,
                "domain" => domain,
                "interval" => interval
            };

            //insert document
            if let Err(e) = client.db("proddle").collection("operations").insert_one(document, None) {
                panic!("failed to upload operations document: {}", e);
            }
        } else if let Some(matches) = matches.subcommand_matches("delete") {
            unimplemented!();
        } else if let Some(matches) = matches.subcommand_matches("search") {
            let domain = matches.value_of("DOMAIN").unwrap();

            match proddle::find_operations(client.clone(), Some(domain), None, None, true) {
                Ok(cursor) => {
                    for document in cursor {
                        println!("{:?}", document);
                    }
                },
                Err(e) => panic!("failed to find operations: {}", e),
            }
        }
    }
}
