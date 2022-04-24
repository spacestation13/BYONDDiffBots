#![feature(proc_macro_hygiene, decl_macro)]

mod git_operations;
mod github_api;
mod github_processor;
mod github_types;
mod job;
mod job_processor;
mod rendering;

#[macro_use]
extern crate rocket;

use std::fs::File;
use std::io::Read;
use std::path::PathBuf;

use lazy_static::lazy_static;
use rocket::fs::FileServer;
use rocket::tokio::runtime::Handle;
use rocket::tokio::sync::RwLock;

#[get("/")]
async fn index() -> &'static str {
    "MDB says hello!"
}

#[derive(Default, Debug)]
pub struct Config {
    pub file_hosting_url: String,
    pub app_id: u64,
}

lazy_static! {
    static ref CONFIG: RwLock<Option<Config>> = RwLock::new(None);
}

fn read_key(path: PathBuf) -> Vec<u8> {
    let mut key_file =
        File::open(&path).unwrap_or_else(|_| panic!("Unable to find file {}", path.display()));

    let mut key = Vec::new();
    let _ = key_file
        .read_to_end(&mut key)
        .unwrap_or_else(|_| panic!("Failed to read key {}", path.display()));

    key
}

// #[post("/payload", data = "<data>")]
// fn payload(data: String) -> &'static str {
//     println!("{}", data);
//     "MDB says hello!"
// }

#[launch]
async fn rocket() -> _ {
    let rocket = rocket::build();
    let figment = rocket.figment();

    let private_key_path: String = figment
        .extract_inner("private_key_path")
        .expect("private_key_path missing from Rocket.toml");

    let file_hosting_url: String = figment
        .extract_inner("file_hosting_url")
        .expect("file_hosting_url missing from Rocket.toml");

    let app_id: u64 = figment
        .extract_inner("app_id")
        .expect("app_id missing from Rocket.toml");

    let key = read_key(PathBuf::from(&private_key_path));

    octocrab::initialise(octocrab::OctocrabBuilder::new().app(
        app_id.into(),
        jsonwebtoken::EncodingKey::from_rsa_pem(&key).unwrap(),
    ))
    .expect("fucked up octocrab");

    let handle = Handle::current();

    CONFIG.write().await.replace(Config {
        file_hosting_url,
        app_id,
    });

    let (job_sender, job_receiver) = flume::unbounded();
    std::thread::spawn(move || {
        handle.spawn(async move { job_processor::handle_jobs(job_receiver).await })
    });

    rocket
        .manage(job::JobSender(job_sender))
        .mount(
            "/",
            routes![index, github_processor::process_github_payload],
        )
        .mount("/images", FileServer::from("./images"))
}
