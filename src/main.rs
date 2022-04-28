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
use std::sync::Arc;

use once_cell::sync::OnceCell;
use rocket::figment::Figment;
use rocket::fs::FileServer;
use rocket::tokio::sync::Mutex;

#[get("/")]
async fn index() -> &'static str {
    "MDB says hello!"
}

#[derive(Default, Debug)]
pub struct Config {
    pub private_key_path: String,
    pub file_hosting_url: String,
    pub app_id: u64,
    pub blacklist: Vec<u64>,
    pub blacklist_contact: String,
}

// lazy_static! {
//     static ref CONFIG: RwLock<Option<Config>> = RwLock::new(None);
// }

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

// #[post("/payload", data = "<data>")]
// fn payload(data: String) -> &'static str {
//     println!("{}", data);
//     "MDB says hello!"
// }

fn set_config(figment: &Figment) -> &Config {
    let private_key_path: String = figment
        .extract_inner("private_key_path")
        .expect("private_key_path missing from Rocket.toml");

    let file_hosting_url: String = figment
        .extract_inner("file_hosting_url")
        .expect("file_hosting_url missing from Rocket.toml");

    let app_id: u64 = figment
        .extract_inner("app_id")
        .expect("app_id missing from Rocket.toml");

    let blacklist: Vec<u64> = figment
        .extract_inner("blacklist")
        .expect("blacklist missing from Rocket.toml");

    let blacklist_contact: String = figment
        .extract_inner("blacklist_contact")
        .expect("blacklist_contact missing from Rocket.toml");

    let config = Config {
        private_key_path,
        file_hosting_url,
        app_id,
        blacklist,
        blacklist_contact,
    };

    CONFIG.set(config).expect("Failed to set config");
    CONFIG.get().unwrap()
}

#[launch]
async fn rocket() -> _ {
    let rocket = rocket::build();
    let figment = rocket.figment();

    let config = set_config(figment);

    let key = read_key(PathBuf::from(&config.private_key_path));

    octocrab::initialise(octocrab::OctocrabBuilder::new().app(
        config.app_id.into(),
        jsonwebtoken::EncodingKey::from_rsa_pem(&key).unwrap(),
    ))
    .expect("fucked up octocrab");

    let journal = Arc::new(Mutex::new(
        job::JobJournal::from_file("jobs.json").await.unwrap(),
    ));

    let (job_sender, job_receiver) = flume::unbounded();
    let journal_clone = journal.clone();

    rocket::tokio::spawn(
        async move { job_processor::handle_jobs(job_receiver, journal_clone).await },
    );

    rocket
        .manage(job::JobSender(job_sender))
        .manage(journal)
        .mount(
            "/",
            routes![index, github_processor::process_github_payload],
        )
        .mount("/images", FileServer::from("./images"))
}
