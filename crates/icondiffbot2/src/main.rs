#![allow(non_snake_case)]

mod github_processor;
mod job_processor;

use std::{fs::File, io::Read, path::PathBuf, sync::Arc};

use diffbot_lib::job::{
    runner::handle_jobs,
    types::{JobJournal, JobSender},
};
use octocrab::OctocrabBuilder;
use once_cell::sync::OnceCell;
// use dmm_tools::dmi::IconFile;
use rocket::{figment::Figment, fs::FileServer, get, launch, routes};
use serde::Deserialize;
use tokio::sync::Mutex;

#[get("/")]
async fn index() -> &'static str {
    "IDB says hello!"
}

#[derive(Debug, Deserialize)]
pub struct Config {
    pub private_key_path: String,
    pub app_id: u64,
    // pub blacklist: Vec<u64>,
    // pub blacklist_contact: String,
}

static CONFIG: OnceCell<Config> = OnceCell::new();

fn init_config(figment: &Figment) -> &Config {
    let config: Config = figment
        .extract()
        .expect("Missing config values in Rocket.toml");

    CONFIG.set(config).expect("Failed to set config");
    CONFIG.get().unwrap()
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

#[launch]
async fn rocket() -> _ {
    let rocket = rocket::build();
    let config = init_config(rocket.figment());

    let key = read_key(PathBuf::from(&config.private_key_path));

    octocrab::initialise(OctocrabBuilder::new().app(
        config.app_id.into(),
        jsonwebtoken::EncodingKey::from_rsa_pem(&key).unwrap(),
    ))
    .expect("Octocrab failed to initialise");

    let journal = Arc::new(Mutex::new(
        JobJournal::from_file("jobs.json").await.unwrap(),
    ));

    tokio::fs::create_dir_all("./images").await.unwrap();

    let (job_sender, job_receiver) = flume::unbounded();
    let journal_clone = journal.clone();

    rocket::tokio::spawn(async move {
        handle_jobs(job_receiver, journal_clone, job_processor::do_job).await
    });

    rocket
        .manage(JobSender(job_sender))
        .manage(journal)
        .mount(
            "/",
            routes![index, github_processor::process_github_payload],
        )
        .mount("/images", FileServer::from("./images"))
}
