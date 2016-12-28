#[macro_use(bson, doc)]
extern crate bson;
extern crate capnp;
extern crate mongodb;

pub mod proddle_capnp {
    include!(concat!(env!("OUT_DIR"), "/proddle_capnp.rs"));
}

use bson::Bson;
use bson::ordered::OrderedDocument;
use mongodb::{Client, ClientInner, ThreadedClient};
use mongodb::coll::options::{CursorType, FindOptions};
use mongodb::cursor::Cursor;
use mongodb::db::ThreadedDatabase;

use std::collections::BTreeMap;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

/*
 * HASHING
 */
pub fn hash_string(value: &str) -> u64 {
    let mut hasher = DefaultHasher::new();
    value.hash(&mut hasher);
    hasher.finish()
}

pub fn get_bucket_key(map: &BTreeMap<u64, DefaultHasher>, key: u64) -> Option<u64> {
    let mut bucket_key = 0;
    for map_key in map.keys() {
        if *map_key > key {
            break;
        }

        bucket_key = *map_key;
    }

    Some(bucket_key)
}

/*
 *
 */
pub struct Module {
    pub timestamp: Option<u64>,
    pub name: String,
    pub version: u16,
    pub dependencies: Option<Vec<String>>,
    pub content: Option<String>,
}

impl Module {
    pub fn new(timestamp: Option<u64>, name: String, version: u16, dependencies: Option<Vec<String>>, content: Option<String>) -> Module {
        Module {
            timestamp: timestamp,
            name: name,
            version: version,
            dependencies: dependencies,
            content: content,
        }
    }

    pub fn from_mongodb(document: &OrderedDocument) -> Result<Module, String> {
        let timestamp = match document.get("timestamp") {
            Some(&Bson::I64(timestamp)) => Some(timestamp as u64),
            _ => return Err("failed to parse timestamp as i64".to_owned()),
        };

        let module_name = match document.get("name") {
            Some(&Bson::String(ref name)) => name.to_owned(),
            _ => return Err("failed to parse name as string".to_owned()),
        };

        let version = match document.get("version") {
            Some(&Bson::I32(version)) => version as u16,
            _ => return Err("failed to parse version as i32".to_owned()),
        };

        let dependencies: Option<Vec<String>> = match document.get("dependencies") {
            Some(&Bson::Array(ref dependencies)) => Some(dependencies.iter().map(|x| x.to_string()).collect()),
            _ => return Err("failed to parse dependencies as array".to_owned()),
        };
        
        let content = match document.get("content") {
            Some(&Bson::String(ref content)) => Some(content.to_owned()),
            _ => return Err("failed to parse content as string".to_owned()),
        };

        Ok(
            Module {
                timestamp: timestamp,
                name: module_name,
                version: version,
                dependencies: dependencies,
                content: content,
            }
        )
    }

    pub fn from_capnproto() -> Result<Module, String> {
        unimplemented!();
    }
}

/*
 * CAPNPROTO BUILDERS
 */
pub fn build_module<'a>(msg: &'a mut proddle_capnp::module::Builder, timestamp: Option<u64>, name: &str, version: u16, dependencies: Option<Vec<&str>>, content: Option<&str>) -> Result<(), String> {
    if let Some(timestamp) = timestamp {
        msg.set_timestamp(timestamp);
    }

    msg.set_name(name);
    msg.set_version(version);

    if let Some(_) = dependencies {
        //TODO set dependencies
        /*let mut msg_dependencies = msg.init_dependencies(dependencies.len() as u32);
        for (i, dependency) in dependencies.iter().enumerate() {
            msg_dependencies.set(i as u32, dependency);
        }*/
    }

    if let Some(content) = content {
        msg.set_content(content);
    }

    Ok(())
}

/*
 * MONGODB
 */
pub fn get_mongodb_client(host: &str, port: u16) -> Result<Client, mongodb::Error> {
    Client::connect(host, port)
}

pub fn find_module(client: Arc<ClientInner>, module_name: &str, version: Option<i32>, order_by_version: bool) -> Result<Option<OrderedDocument>, mongodb::Error> {
    //create search document
    let search_document = match version {
        Some(search_version) => Some(doc! { "name" => module_name, "version" => search_version }),
        None => Some(doc! { "name" => module_name }),
    };

    //create find options
    let negative_one = -1;
    let sort_document = match order_by_version {
        true => Some(doc! { "version" => negative_one }),
        false => None,
    };

    let find_options = Some(FindOptions {
        allow_partial_results: false,
        no_cursor_timeout: false,
        op_log_replay: false,
        skip: 0,
        limit: 1,
        cursor_type: CursorType::NonTailable,
        batch_size: 0,
        comment: None,
        max_time_ms: None,
        modifiers: None,
        projection: None,
        sort: sort_document,
        read_preference: None,
    });

    //execute find one
    client.db("proddle").collection("modules").find_one(search_document, find_options)
}

pub fn find_modules(client: Arc<ClientInner>, module_name: Option<&str>, version: Option<i32>, limit: Option<i32>, order_by_version: bool) -> Result<Cursor, mongodb::Error> {
    //create search document
    let search_document = match module_name {
        Some(search_module_name) => {
            match version {
                Some(search_version) => Some(doc! { "name" => search_module_name, "version" => search_version }),
                None => Some(doc! { "name" => search_module_name }),
            }
        },
        None => {
            match version {
                Some(search_version) => Some(doc! { "version" => search_version }),
                None => None,
            }
        },
    };

    //create find options
    let negative_one = -1;
    let sort_document = match order_by_version {
        true => Some(doc! { "version" => negative_one }),
        false => None,
    };

    let find_options = Some(FindOptions {
        allow_partial_results: false,
        no_cursor_timeout: false,
        op_log_replay: false,
        skip: 0,
        limit: limit.unwrap_or(0),
        cursor_type: CursorType::NonTailable,
        batch_size: 0,
        comment: None,
        max_time_ms: None,
        modifiers: None,
        projection: None,
        sort: sort_document,
        read_preference: None,
    });

    //execute find
    client.db("proddle").collection("modules").find(search_document, find_options)
}

pub fn find_operation(client: Arc<ClientInner>, domain: &str, module_name: Option<&str>, order_by_timestamp: bool) -> Result<Option<OrderedDocument>, mongodb::Error> {
    //create search document
    let search_document = match module_name {
        Some(search_module_name) => Some(doc! { "domain" => domain, "module" => search_module_name }),
        None => Some(doc! { "domain" => domain }),
    };

    //create find options
    let negative_one = -1;
    let sort_document = match order_by_timestamp {
        true => Some(doc! { "timestamp" => negative_one }),
        false => Some(doc! { "_id" => 1 }),
    };

    let find_options = Some(FindOptions {
        allow_partial_results: false,
        no_cursor_timeout: false,
        op_log_replay: false,
        skip: 0,
        limit: 1,
        cursor_type: CursorType::NonTailable,
        batch_size: 0,
        comment: None,
        max_time_ms: None,
        modifiers: None,
        projection: None,
        sort: sort_document,
        read_preference: None,
    });

    //execute find one
    client.db("proddle").collection("operations").find_one(search_document, find_options)
}

pub fn find_operations(client: Arc<ClientInner>, domain: Option<&str>, module_name: Option<&str>, limit: Option<i32>, order_by_timestamp: bool) -> Result<Cursor, mongodb::Error> {
    //create search document
    let search_document = match domain {
        Some(domain) => {
            match module_name {
                Some(search_module_name) => Some(doc! { "domain" => domain, "module" => search_module_name }),
                None => Some(doc! { "domain" => domain }),
            }
        },
        None => {
            match module_name {
                Some(search_module_name) => Some(doc! { "module" => search_module_name }),
                None => None,
            }
        }
    };

    //create find options
    let negative_one = -1;
    let sort_document = match order_by_timestamp {
        true => Some(doc! { "timestamp" => negative_one }),
        false => Some(doc! { "_id" => 1 }),
    };

    let find_options = Some(FindOptions {
        allow_partial_results: false,
        no_cursor_timeout: false,
        op_log_replay: false,
        skip: 0,
        limit: limit.unwrap_or(0),
        cursor_type: CursorType::NonTailable,
        batch_size: 0,
        comment: None,
        max_time_ms: None,
        modifiers: None,
        projection: None,
        sort: sort_document,
        read_preference: None,
    });

    //specify operations collection
    client.db("proddle").collection("operations").find(search_document, find_options)
}
