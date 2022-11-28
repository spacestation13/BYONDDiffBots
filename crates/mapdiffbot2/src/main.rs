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

fn init_config(path: &std::path::Path) -> eyre::Result<&'static Config> {
    let mut config_str = String::new();
    File::open(path)?.read_to_string(&mut config_str)?;

    let config = toml::from_str(&config_str)?;

    CONFIG.set(config).expect("Failed to set config");
    Ok(CONFIG.get().unwrap())
}

const JOB_JOURNAL_LOCATION: &str = "jobs";

#[launch]
fn rocket() -> _ {
    stable_eyre::install().expect("Eyre handler installation failed!");

    let config_path = std::path::Path::new(".").join("config.toml");
    let config =
        init_config(&config_path).unwrap_or_else(|_| panic!("Failed to read {:?}", config_path));

    diffbot_lib::logger::init_logger(&config.logging).expect("Log init failed!");

    let key = read_key(PathBuf::from(&config.private_key_path));

    octocrab::initialise(octocrab::OctocrabBuilder::new().app(
        config.app_id.into(),
        jsonwebtoken::EncodingKey::from_rsa_pem(&key).unwrap(),
    ))
    .expect("fucked up octocrab");

    let (job_sender, job_receiver) = yaque::channel(JOB_JOURNAL_LOCATION)
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
                        let job =
                            serde_json::to_vec(&JobType::CleanupJob("GC_REQUEST_DUMMY".to_owned()))
                                .expect("Cannot serialize cleanupjob, what the fuck");
                        async move {
                            if let Err(err) = sender_clone.lock().await.send(job).await {
                                error!("Cannot send cleanup job: {}", err)
                            }
                        }
                    })
                    .expect("Can't create Cron task"),
            )
            .expect("cannot add cron job, FUCK");
        rocket::tokio::signal::ctrl_c()
            .await
            .expect("Cannot wait for sigterm");
        scheduler.remove_task(1).expect("Can't remove task");
        scheduler
            .stop_delay_timer()
            .expect("Can't stop delaytimer, FUCK");
    });

    rocket::build()
        .manage(job_sender)
        .mount(
            "/",
            routes![index, github_processor::process_github_payload],
        )
        .mount("/images", FileServer::from("./images"))
}
