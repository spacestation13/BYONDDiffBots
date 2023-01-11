use crate::github::github_types::{
    CreateCheckRun, Output, RawCheckRun, Repository, UpdateCheckRunBuilder,
};
use async_fs::File;
use eyre::{format_err, Context, Result};
use futures_lite::io::AsyncWriteExt;
use octocrab::models::repos::Content;
use octocrab::models::InstallationId;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::{future::Future, pin::Pin};

pub struct GithubEvent(pub String, pub Option<String>);

impl actix_web::FromRequest for GithubEvent {
    type Error = std::io::Error;

    type Future = Pin<Box<dyn Future<Output = Result<Self, Self::Error>>>>;

    fn from_request(req: &actix_web::HttpRequest, _: &mut actix_web::dev::Payload) -> Self::Future {
        let req = req.clone();
        Box::pin(async move {
            let event_header = match req.headers().get("X-Github-Event") {
                Some(event) => event
                    .to_str()
                    .map_err(|_| {
                        std::io::Error::new(
                            std::io::ErrorKind::InvalidData,
                            "Corrupt X-Github-Event header, failed to convert to string",
                        )
                    })?
                    .to_owned(),
                None => {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        "Missing X-Github-Event header",
                    ))
                }
            };
            let hmac_header = match req.headers().get("X-Hub-Signature-256") {
                Some(event) => Some(
                    event
                        .to_str()
                        .map_err(|_| {
                            std::io::Error::new(
                                std::io::ErrorKind::InvalidData,
                                "Corrupt X-Hub-Signature-256 header, failed to convert to string",
                            )
                        })?
                        .to_owned(),
                ),
                _ => None,
            };
            Ok(GithubEvent(event_header, hmac_header))
        })
    }
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct CheckRun {
    id: u64,
    installation_id: InstallationId,
    head_sha: String,
    repo: String,
}

impl CheckRun {
    pub async fn create<I: Into<InstallationId>>(
        full_repo: &str,
        head_sha: &str,
        inst_id: I,
        name: Option<&str>,
    ) -> Result<Self> {
        let inst_id = inst_id.into();
        let result: RawCheckRun = octocrab::instance()
            .installation(inst_id)
            .post(
                format!("/repos/{full_repo}/check-runs"),
                Some(&CreateCheckRun {
                    name: name.unwrap_or("BYONDDiffBot").to_string(),
                    head_sha: head_sha.to_string(),
                }),
            )
            .await
            .context("Submitting check")?;

        Ok(Self {
            id: result.id,
            installation_id: inst_id,
            head_sha: head_sha.to_string(),
            repo: full_repo.to_owned(),
        })
    }

    /// Creates a new check run for the same PR
    pub async fn duplicate(&self, name: &str) -> Result<Self> {
        Self::create(&self.repo, &self.head_sha, self.installation_id, Some(name)).await
    }

    pub async fn rename(&self, name: &str) -> Result<()> {
        self.update(UpdateCheckRunBuilder::default().name(name.to_owned()))
            .await
            .context("Renaming check run")
    }

    pub async fn mark_queued(&self) -> Result<()> {
        self.update(
            UpdateCheckRunBuilder::default()
                .status("queued")
                .started_at(chrono::Utc::now().to_rfc3339()),
        )
        .await
        .context("Marking check run as queued")
    }

    pub async fn mark_started(&self) -> Result<()> {
        self.update(
            UpdateCheckRunBuilder::default()
                .status("in_progress")
                .started_at(chrono::Utc::now().to_rfc3339()),
        )
        .await
        .context("Marking check run as in progress")
    }

    pub async fn mark_failed(&self, stack_trace: &str) -> Result<()> {
        let summary = format!(
            include_str!("error_template.txt"),
            stack_trace = stack_trace
        );

        self.update(
            UpdateCheckRunBuilder::default()
                .status("completed")
                .conclusion("failure")
                .completed_at(chrono::Utc::now().to_rfc3339())
                .output(Output {
                    title: "Error handling job",
                    summary,
                    text: "".to_owned(),
                }),
        )
        .await
        .context("Marking check as failure")
    }

    pub async fn mark_succeeded(&self, output: Output) -> Result<()> {
        self.update(
            UpdateCheckRunBuilder::default()
                .conclusion("success")
                .completed_at(chrono::Utc::now().to_rfc3339())
                .output(output),
        )
        .await
        .context("Marking check as success")
    }

    pub async fn mark_skipped(&self, output: Output) -> Result<()> {
        self.update(
            UpdateCheckRunBuilder::default()
                .conclusion("skipped")
                .completed_at(chrono::Utc::now().to_rfc3339())
                .output(output),
        )
        .await
        .context("Marking check as skipped")
    }

    pub async fn set_output(&self, output: Output) -> Result<()> {
        self.update(UpdateCheckRunBuilder::default().output(output))
            .await
            .context("Setting check run output")
    }

    async fn update(&self, builder: UpdateCheckRunBuilder) -> Result<()> {
        let update = builder.build().context("Building UpdateCheckRun")?;

        #[derive(Deserialize)]
        struct Empty {}
        let _: Empty = octocrab::instance()
            .installation(self.installation_id)
            .patch(
                format!(
                    "/repos/{repo}/check-runs/{check_run_id}",
                    repo = self.repo,
                    check_run_id = self.id,
                ),
                Some(&update),
            )
            .await
            .context("Updating check run")?;

        Ok(())
    }

    pub fn id(&self) -> u64 {
        self.id
    }
}

static DOWNLOAD_DIR: &str = "download";

async fn find_content<S: AsRef<str>>(
    installation: &InstallationId,
    repo: &Repository,
    filename: S,
    commit: S,
) -> Result<Content> {
    let (owner, repo) = repo.name_tuple();
    let items = octocrab::instance()
        .installation(*installation)
        .repos(owner, repo)
        .get_content()
        .path(filename.as_ref())
        .r#ref(commit.as_ref())
        .send()
        .await?
        .take_items();

    if items.len() > 1 {
        return Err(format_err!("Directory given to find_content"));
    }

    items
        .into_iter()
        .next()
        .ok_or_else(|| format_err!("No content was found"))
}

pub async fn download_url<S: AsRef<str>>(
    installation: &InstallationId,
    repo: &Repository,
    filename: S,
    commit: S,
) -> Result<Vec<u8>> {
    let target = find_content(installation, repo, filename, commit).await?;

    let download_url = target
        .download_url
        .as_ref()
        .ok_or_else(|| format_err!("No download URL given by GitHub"))?;

    let response = reqwest::get(download_url).await?;

    Ok(response.bytes().await?.to_vec())
}

pub async fn download_file<S: AsRef<str>>(
    installation: &InstallationId,
    repo: &Repository,
    filename: S,
    commit: S,
) -> Result<PathBuf> {
    let target = find_content(installation, repo, &filename, &commit).await?;

    let mut path = PathBuf::new();
    path.push(".");
    path.push(DOWNLOAD_DIR);
    path.push(&target.sha);
    path.set_extension("dmi"); // Method should have an IDB qualifier due to being a shared crate

    async_fs::create_dir_all(path.parent().unwrap()).await?;
    let mut file = File::create(&path).await?;

    let data = download_url(installation, repo, &filename, &commit).await?;
    file.write_all(&data).await?;
    Ok(path)
}
