#![feature(proc_macro_hygiene, decl_macro)]

mod git_operations;
mod github_processor;
mod github_types;
mod job;
mod job_processor;
mod render_error;
mod rendering;

#[macro_use]
extern crate rocket;

use lazy_static::lazy_static;
use rocket::fs::FileServer;
use rocket::tokio::runtime::Handle;
use rocket::tokio::sync::RwLock;
use serde::Deserialize;

#[derive(Deserialize, Debug)]
struct AssBalls {
    name: String,
}

#[get("/")]
async fn index() -> &'static str {
    let ass: AssBalls = octocrab::instance()
        .get("/app", None::<&()>)
        .await
        .expect("Could not get app");
    println!("{:?}", ass);
    "MDB says hello!"
}

#[derive(Default, Debug)]
pub struct Config {
    pub file_hosting_url: String,
}

lazy_static! {
    static ref CONFIG: RwLock<Option<Config>> = RwLock::new(None);
}

// #[post("/payload", data = "<data>")]
// fn payload(data: String) -> &'static str {
//     println!("{}", data);
//     "MDB says hello!"
// }

#[launch]
async fn rocket() -> _ {
    let key = include_bytes!("../mapdiffbot2.pem");

    octocrab::initialise(octocrab::OctocrabBuilder::new().app(
        192759.into(),
        jsonwebtoken::EncodingKey::from_rsa_pem(key).unwrap(),
    ))
    .expect("fucked up octocrab");

    let handle = Handle::current();

    let rocket = rocket::build();
    let figment = rocket.figment();

    let file_url: String = figment
        .extract_inner("file_hosting_url")
        .expect("file_hosting_url");

    CONFIG.write().await.replace(Config {
        file_hosting_url: file_url,
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
