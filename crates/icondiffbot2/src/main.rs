mod github_processor;
mod job_processor;
mod sha;
mod table_builder;

use diffbot_lib::job::{
    runner::handle_jobs,
    types::{JobJournal, JobSender},
};
use octocrab::OctocrabBuilder;
use once_cell::sync::OnceCell;
use serde::Deserialize;
use std::{
    fs::File,
    io::Read,
    path::{Path, PathBuf},
    sync::Arc,
};
use tokio::sync::Mutex;

#[actix_web::get("/")]
async fn index() -> &'static str {
    "IDB says hello!"
}

pub type DataJobSender = actix_web::web::Data<JobSender>;
pub type DataJobJournal = actix_web::web::Data<Mutex<JobJournal>>;

#[derive(Debug, Deserialize)]
pub struct GithubConfig {
    pub app_id: u64,
    pub private_key_path: String,
}

#[derive(Debug, Deserialize)]
pub struct WebLimitsConfig {
    pub forms: usize,
    pub string: usize,
}

#[derive(Debug, Deserialize)]
pub struct WebConfig {
    pub address: String,
    pub port: u16,
    pub file_hosting_url: String,
    pub limits: WebLimitsConfig,
}

#[derive(Debug, Deserialize)]
pub struct Config {
    pub github: GithubConfig,
    pub web: WebConfig,
}

static CONFIG: OnceCell<Config> = OnceCell::new();
// static FLAME_LAYER_GUARD: OnceCell<tracing_flame::FlushGuard<std::io::BufWriter<File>>> =
// OnceCell::new();

fn init_config(path: &Path) -> anyhow::Result<&'static Config> {
    let mut config_str = String::new();
    File::open(path)?.read_to_string(&mut config_str)?;

    let config = toml::from_str(&config_str)?;

    CONFIG.set(config).expect("Failed to set config");
    Ok(CONFIG.get().unwrap())
}

// fn init_global_subscriber() {
//     use tracing_subscriber::prelude::*;

//     let fmt_layer = tracing_subscriber::fmt::Layer::default();

//     // let (flame_layer, guard) = tracing_flame::FlameLayer::with_file("./tracing.folded").unwrap();

//     tracing_subscriber::registry()
//         .with(fmt_layer)
//         // .with(flame_layer)
//         .init();

//     // FLAME_LAYER_GUARD
//     //     .set(guard)
//     //     .expect("Failed to store flame layer guard");
// }

fn read_key(path: &Path) -> Vec<u8> {
    let mut key_file =
        File::open(&path).unwrap_or_else(|_| panic!("Unable to find file {}", path.display()));

    let mut key = Vec::new();
    let _ = key_file
        .read_to_end(&mut key)
        .unwrap_or_else(|_| panic!("Failed to read key {}", path.display()));

    key
}

#[tokio::main]
async fn main() -> std::io::Result<()> {
    // init_global_subscriber();

    let config_path = Path::new(".").join("config.toml");
    let config =
        init_config(&config_path).unwrap_or_else(|_| panic!("Failed to read {:?}", config_path));

    let key = read_key(&PathBuf::from(&config.github.private_key_path));

    octocrab::initialise(OctocrabBuilder::new().app(
        config.github.app_id.into(),
        jsonwebtoken::EncodingKey::from_rsa_pem(&key).unwrap(),
    ))
    .expect("Octocrab failed to initialise");

    let journal = Arc::new(Mutex::new(
        JobJournal::from_file("jobs.json").await.unwrap(),
    ));

    tokio::fs::create_dir_all("./images").await.unwrap();

    let (job_sender, job_receiver) = flume::unbounded();

    let journal_clone = journal.clone();
    tokio::spawn(async move {
        handle_jobs(
            "IconDiffBot2",
            job_receiver,
            journal_clone,
            job_processor::do_job,
        )
        .await
    });

    let journal: DataJobJournal = journal.into();
    let job_sender: DataJobSender = actix_web::web::Data::new(JobSender(job_sender));

    actix_web::HttpServer::new(move || {
        let form_config = actix_web::web::FormConfig::default().limit(config.web.limits.forms);
        let string_config =
            actix_web::web::PayloadConfig::default().limit(config.web.limits.string);
        actix_web::App::new()
            .app_data(form_config)
            .app_data(string_config)
            .app_data(journal.clone())
            .app_data(job_sender.clone())
            .service(index)
            .service(github_processor::process_github_payload_actix)
            .service(actix_files::Files::new("/images", "./images"))
    })
    .bind((config.web.address.as_ref(), config.web.port))?
    .run()
    .await
}
