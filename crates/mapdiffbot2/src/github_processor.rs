use diffbot_lib::log;
use eyre::{Context, Result};
use octocrab::models::InstallationId;

use crate::DataJobSender;
use diffbot_lib::{
    github::{
        github_api::CheckRun,
        github_types::{
            ChangeType, Installation, Output, PullRequest, PullRequestEventPayload, Repository,
        },
        graphql::get_pull_files,
    },
    job::types::{Job, JobType},
};

async fn process_pull(
    repo: Repository,
    pull: PullRequest,
    check_run: CheckRun,
    installation: &Installation,
    job_sender: DataJobSender,
) -> Result<()> {
    log::trace!("Processing pull request");

    if pull
        .title
        .as_ref()
        .ok_or_else(|| eyre::anyhow!("PR title is None"))?
        .to_ascii_lowercase()
        .contains("[mdb ignore]")
    {
        let output = Output {
            title: "PR Ignored",
            summary: "This PR has `[MDB IGNORE]` in the title. Aborting.".to_owned(),
            text: "".to_owned(),
        };

        check_run.mark_skipped(output).await?;

        return Ok(());
    }

    let (blacklist, contact) = {
        let conf = &crate::CONFIG.get().unwrap();
        (&conf.blacklist, &conf.blacklist_contact)
    };

    if blacklist.contains(&repo.id) {
        let output = Output {
            title: "Repo blacklisted",
            summary: format!(
                "Repository {} is blacklisted. {}",
                repo.full_name(),
                contact
            ),
            text: "".to_owned(),
        };

        check_run.mark_skipped(output).await?;

        return Ok(());
    }

    let files = get_pull_files(repo.name_tuple(), installation.id, &pull)
        .await
        .context("Getting files modified by PR")?
        .into_iter()
        .filter(|f| f.filename.ends_with(".dmm"))
        .filter(|f| {
            matches!(
                f.status,
                ChangeType::Added | ChangeType::Deleted | ChangeType::Modified
            )
        })
        .collect::<Vec<_>>();

    if files.is_empty() {
        let output = Output {
            title: "No map changes",
            summary: "There are no relevant changed map files to render.".to_owned(),
            text: "".to_owned(),
        };

        check_run.mark_skipped(output).await?;

        return Ok(());
    }

    check_run.mark_queued().await?;

    let job = Job {
        repo,
        base: pull.base,
        head: pull.head,
        pull_request: pull.number,
        files,
        check_run,
        installation: InstallationId(installation.id),
    };

    let job = serde_json::to_vec(&JobType::GithubJob(job))?;

    job_sender.lock().await.send(job).await?;

    log::trace!("Job sent to queue");

    Ok(())
}

use std::{future::Future, pin::Pin};

#[derive(Debug)]
pub struct GithubEvent(pub String);

impl actix_web::FromRequest for GithubEvent {
    type Error = std::io::Error;

    type Future = Pin<Box<dyn Future<Output = Result<Self, Self::Error>>>>;

    fn from_request(req: &actix_web::HttpRequest, _: &mut actix_web::dev::Payload) -> Self::Future {
        let req = req.clone();
        Box::pin(async move {
            match req.headers().get("X-Github-Event") {
                Some(event) => Ok(GithubEvent(event.to_str().unwrap().to_owned())),
                None => Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "Missing X-Github-Event header",
                )),
            }
        })
    }
}

async fn handle_pull_request(payload: String, job_sender: DataJobSender) -> Result<&'static str> {
    let payload: PullRequestEventPayload = serde_json::from_str(&payload)?;
    if payload.action != "opened" && payload.action != "synchronize" {
        return Ok("PR not opened or updated");
    }

    log::trace!("Creating checkrun");

    let check_run = CheckRun::create(
        &payload.repository.full_name(),
        &payload.pull_request.head.sha,
        payload.installation.id,
        None,
    )
    .await?;

    process_pull(
        payload.repository,
        payload.pull_request,
        check_run,
        &payload.installation,
        job_sender,
    )
    .await?;

    Ok("Check submitted")
}

#[actix_web::post("/payload")]
pub async fn process_github_payload(
    event: GithubEvent,
    payload: String,
    job_sender: DataJobSender,
) -> actix_web::Result<&'static str> {
    if event.0 != "pull_request" {
        return Ok("Not a pull request event");
    }

    log::trace!("Payload received, processing");

    handle_pull_request(payload, job_sender).await.map_err(|e| {
        log::error!("Error handling event: {:?}", e);
        actix_web::error::ErrorBadRequest(e)
    })
}
