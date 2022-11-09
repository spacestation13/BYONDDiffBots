mod git_operations;
pub(crate) mod github_processor;
pub(crate) mod job_processor;
pub(crate) mod rendering;
pub(crate) mod runner;

#[macro_use]
extern crate rocket;

use std::fs::File;
use std::io::Read;
use std::path::PathBuf;

use diffbot_lib::job::types::JobType;
use once_cell::sync::OnceCell;
use rocket::figment::Figment;
use rocket::fs::FileServer;
use serde::Deserialize;
use std::sync::Arc;

#[get("/")]
async fn index() -> &'static str {
    "MDB says hello!"
}

#[derive(Debug, Deserialize)]
pub struct Config {
    pub private_key_path: String,
    pub file_hosting_url: String,
    pub app_id: u64,
    pub blacklist: std::collections::HashSet<u64>,
    pub blacklist_contact: String,
    pub gc_schedule: String,
    pub logging: String,
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

const JOB_JOURNAL_LOCATION: &str = "/jobs";

#[launch]
async fn rocket() -> _ {
    let curr_dir = std::env::current_exe().unwrap();
    let parent_curr = curr_dir.parent().unwrap();
    let queue_dir: PathBuf = [parent_curr, JOB_JOURNAL_LOCATION.as_ref()]
        .iter()
        .collect();

    std::env::set_current_dir(parent_curr).unwrap();

    stable_eyre::install().expect("Eyre handler installation failed!");
    diffbot_lib::logger::init_logger("info").expect("Log init failed!");

    let rocket = rocket::build();
    let config = init_config(rocket.figment());

    let key = read_key(PathBuf::from(&config.private_key_path));

    octocrab::initialise(octocrab::OctocrabBuilder::new().app(
        config.app_id.into(),
        jsonwebtoken::EncodingKey::from_rsa_pem(&key).unwrap(),
    ))
    .expect("fucked up octocrab");

    let (job_sender, job_receiver) = yaque::channel(queue_dir)
        .expect("Couldn't open an on-disk queue, check permissions or drive space?");

    rocket::tokio::spawn(runner::handle_jobs("MapDiffBot2", job_receiver));

    let job_sender = Arc::new(rocket::tokio::sync::Mutex::new(job_sender));

    let job_clone = job_sender.clone();

    let cron_str = config.gc_schedule.to_owned();

    rocket::tokio::spawn(async move {
        use delay_timer::prelude::*;
        let scheduler = DelayTimerBuilder::default()
            .tokio_runtime_by_default()
            .build();
        scheduler
            .add_task(
                TaskBuilder::default()
                    .set_frequency_repeated_by_cron_str(cron_str.as_str())
                    .set_maximum_parallel_runnable_num(1)
                    .set_task_id(1)
                    .spawn_async_routine(move || {
                        let sender_clone = job_clone.clone();
                        async move {
                            let job = serde_json::to_vec(&JobType::CleanupJob(
                                "GC_REQUEST_DUMMY".to_owned(),
                            ))
                            .expect("Cannot serialize cleanupjob, what the fuck");
                            if let Err(err) = sender_clone.lock().await.send(job).await {
                                error!("Cannot send cleanup job: {}", err)
                            };
                        }
                    })
                    .expect("Can't create Cron task"),
            )
            .expect("cannot add cron job, FUCK");
    });

    rocket
        .manage(job_sender)
        .mount(
            "/",
            routes![index, github_processor::process_github_payload],
        )
        .mount("/images", FileServer::from("./images"))
}
