pub(crate) mod github_processor;
pub(crate) mod job_processor;
pub(crate) mod rendering;

#[macro_use]
extern crate rocket;

use std::fs::File;
use std::io::Read;
use std::path::PathBuf;
use std::sync::Arc;

use once_cell::sync::OnceCell;
use rocket::figment::Figment;
use rocket::fs::FileServer;
use rocket::tokio::sync::Mutex;
use serde::Deserialize;

use diffbot_lib::job::runner::handle_jobs;
use diffbot_lib::job::types::{JobJournal, JobSender};

#[get("/")]
async fn index() -> &'static str {
    "MDB says hello!"
}

#[derive(Debug, Deserialize)]
pub struct Config {
    pub private_key_path: String,
    pub file_hosting_url: String,
    pub app_id: u64,
    pub blacklist: Vec<u64>,
    pub blacklist_contact: String,
}

static CONFIG: OnceCell<Config> = OnceCell::new();

fn read_key(path: PathBuf) -> Vec<u8> {
    let mut key_file =
        File::open(&path).unwrap_or_else(|_| panic!("Unable to find file {}", path.display()));

    let mut key = Vec::new();
    let _ = key_file
        .read_to_end(&mut key)
        .unwrap_or_else(|_| panic!("Failed to read key {}", path.display()));

    key
}

fn init_config(figment: &Figment) -> &Config {
    let config: Config = figment
        .extract()
        .expect("Missing config values in Rocket.toml");

    CONFIG.set(config).expect("Failed to set config");
    CONFIG.get().unwrap()
}

#[launch]
async fn rocket() -> _ {
    let rocket = rocket::build();
    let config = init_config(rocket.figment());

    let key = read_key(PathBuf::from(&config.private_key_path));

    octocrab::initialise(octocrab::OctocrabBuilder::new().app(
        config.app_id.into(),
        jsonwebtoken::EncodingKey::from_rsa_pem(&key).unwrap(),
    ))
    .expect("fucked up octocrab");

    let journal = Arc::new(Mutex::new(
        JobJournal::from_file("jobs.json").await.unwrap(),
    ));

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
