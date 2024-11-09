mod gc_job;
mod git_operations;
mod github_processor;
mod job_processor;
mod rendering;
mod runner;

use std::fs::File;
use std::io::Read;
use std::path::PathBuf;

use mysql_async::prelude::Queryable;
use serde::Deserialize;
use std::sync::OnceLock;

#[cfg(not(target_env = "msvc"))]
#[global_allocator]
static GLOBAL: tikv_jemallocator::Jemalloc = tikv_jemallocator::Jemalloc;

pub type DataJobSender =
    actix_web::web::Data<diffbot_lib::job::types::JobSender<diffbot_lib::job::types::JobType>>;

#[actix_web::get("/")]
async fn index() -> &'static str {
    "MDB says hello!"
}

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
    pub limits: Option<WebLimitsConfig>,
}

#[derive(Debug, Deserialize)]
pub struct AzureBlobs {
    pub storage_account: String,
    pub storage_access_key: String,
    pub storage_container: String,
}

#[derive(Debug, Deserialize)]
pub struct GrafanaLoki {
    url: String,
}

#[derive(Debug, Deserialize)]
pub struct Config {
    pub github: GithubConfig,
    pub web: WebConfig,
    #[serde(default = "std::collections::HashSet::new")]
    pub blacklist: std::collections::HashSet<u64>,
    #[serde(default = "String::new")]
    pub blacklist_contact: String,
    #[serde(default = "default_schedule")]
    pub gc_schedule: String,
    #[serde(default = "default_log_level")]
    pub logging: String,
    #[serde(default = "default_msg")]
    pub summary_msg: String,
    pub secret: Option<String>,
    pub db_url: Option<String>,
    pub azure_blobs: Option<AzureBlobs>,
    pub grafana_loki: Option<GrafanaLoki>,
}

fn default_schedule() -> String {
    "0 0 4 * * *".to_string()
}

fn default_log_level() -> String {
    "info".to_string()
}

fn default_msg() -> String {
    "*Please file any issues [here](https://github.com/spacestation13/BYONDDiffBots/issues).*\n\n*Github may fail to render some images, appearing as cropped on large map changes. Please use the raw links in this case.*\n\nMaps with diff:".to_string()
}

static CONFIG: OnceLock<Config> = OnceLock::new();

fn read_key(path: PathBuf) -> Vec<u8> {
    let mut key_file =
        File::open(&path).unwrap_or_else(|_| panic!("Unable to find file {}", path.display()));

    let mut key = Vec::new();
    _ = key_file
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

fn read_config() -> &'static Config {
    CONFIG.get().unwrap()
}

type Azure = Option<std::sync::Arc<object_store::azure::MicrosoftAzure>>;

#[actix_web::main]
async fn main() -> eyre::Result<()> {
    simple_eyre::install().expect("Eyre handler installation failed!");

    let config_path = std::path::Path::new("./config").join("config.toml");
    let config =
        init_config(&config_path).unwrap_or_else(|_| panic!("Failed to read {config_path:?}"));

    let (layer, tasks) = if let Some(ref loki_config) = config.grafana_loki {
        let (layer, tasks) = tracing_loki::builder()
            .build_url(tracing_loki::url::Url::parse(&loki_config.url).unwrap())?;
        (Some(layer), Some(tasks))
    } else {
        (None, None)
    };

    diffbot_lib::logger::init_logger(&config.logging, layer).expect("Log init failed!");

    if let Some(tasks) = tasks {
        actix_web::rt::spawn(tasks);
    }

    let key = read_key(PathBuf::from(&config.github.private_key_path));

    octocrab::initialise(
        octocrab::OctocrabBuilder::new()
            .app(
                config.github.app_id.into(),
                jsonwebtoken::EncodingKey::from_rsa_pem(&key).unwrap(),
            )
            .build()
            .expect("fucked up octocrab"),
    );

    let (job_sender, job_receiver) = flume::unbounded();

    let pool = config
        .db_url
        .as_ref()
        .map(|url| mysql_async::Pool::new(url.as_str()));

    if let Some(ref pool) = pool {
        let mut conn = pool.get_conn().await?;
        conn.query_drop(
            r"CREATE TABLE IF NOT EXISTS `jobs` (
                `check_id` BIGINT(20) NOT NULL,
                `repo_id` BIGINT(20) NOT NULL,
                `pr_number` INT(11) NOT NULL,
                `merge_date` DATETIME NULL DEFAULT NULL,
                `processed` BIT(1) NOT NULL DEFAULT b'0',
                PRIMARY KEY (`check_id`) USING BTREE,
                INDEX `merge_date` (`processed`) USING BTREE,
                INDEX `processed` (`processed`) USING BTREE
            ) COLLATE='utf8mb4_general_ci' ENGINE=InnoDB;",
        )
        .await?;
    }

    let blob_client = config.azure_blobs.as_ref().map(|azure| {
        std::sync::Arc::new(
            object_store::azure::MicrosoftAzureBuilder::new()
                .with_account(azure.storage_account.clone())
                .with_access_key(azure.storage_access_key.clone())
                .with_container_name(azure.storage_container.clone())
                .with_client_options(
                    object_store::ClientOptions::new()
                        .with_content_type_for_suffix("png", "image/png"),
                )
                .build()
                .expect("Trying to connect to azure"),
        )
    });

    actix_web::rt::spawn(runner::handle_jobs(
        "MapDiffBot2",
        job_receiver,
        blob_client,
    ));

    let job_clone = job_sender.clone();

    let cron_str = config.gc_schedule.to_owned();

    actix_web::rt::spawn(async move { gc_job::gc_scheduler(cron_str, job_clone).await });

    actix_web::HttpServer::new(move || {
        let pool = pool.clone();
        use actix_web::web::{FormConfig, PayloadConfig};
        //absolutely rancid
        let (form_config, string_config) = config.web.limits.as_ref().map_or(
            (FormConfig::default(), PayloadConfig::default()),
            |limits| {
                (
                    FormConfig::default().limit(limits.forms),
                    PayloadConfig::default().limit(limits.string),
                )
            },
        );

        actix_web::App::new()
            .app_data(form_config)
            .app_data(string_config)
            .app_data(actix_web::web::Data::new(job_sender.clone()))
            .app_data(actix_web::web::Data::new(pool))
            .service(index)
            .service(github_processor::process_github_payload)
            .service(actix_files::Files::new("/images", "./images"))
    })
    .bind((config.web.address.as_ref(), config.web.port))?
    .run()
    .await?;
    Ok(())
}
