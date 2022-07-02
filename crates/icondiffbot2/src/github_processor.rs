use std::{future::Future, pin::Pin};

use anyhow::Result;
use diffbot_lib::{
    github::github_api::get_pull_files,
    github::{
        github_api::CheckRun,
        github_types::{Output, PullRequestEventPayload},
    },
    job::types::Job,
};
use octocrab::models::{pulls::FileDiff, InstallationId};

use crate::{DataJobJournal, DataJobSender};

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

async fn handle_pull_request(
    payload: PullRequestEventPayload,
    job_sender: DataJobSender,
    journal: DataJobJournal,
) -> Result<()> {
    match payload.action.as_str() {
        "opened" => {}
        #[cfg(debug_assertions)]
        "reopened" => {}
        "synchronize" => {}
        _ => return Ok(()),
    }

    let check_run = CheckRun::create(
        &payload.pull_request.base.repo.full_name(),
        &payload.pull_request.head.sha,
        payload.installation.id,
        Some("IconDiffBot2"),
    )
    .await?;

    if payload
        .pull_request
        .title
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("PR title is None"))?
        .to_ascii_lowercase()
        .contains("[idb ignore]")
    {
        let output = Output {
            title: "PR Ignored".to_owned(),
            summary: "This PR has `[IDB IGNORE]` in the title. Aborting.".to_owned(),
            text: "".to_owned(),
        };

        check_run.mark_skipped(output).await?;
        return Ok(());
    }

    let files = get_pull_files(&payload.installation, &payload.pull_request).await?;

    let changed_dmis: Vec<FileDiff> = files
        .into_iter()
        .filter(|e| e.filename.ends_with(".dmi"))
        .collect();

    if changed_dmis.is_empty() {
        let output = Output {
            title: "No icon chages".to_owned(),
            summary: "There are no changed icon files to render.".to_owned(),
            text: "".to_owned(),
        };

        check_run.mark_skipped(output).await?;

        return Ok(());
    }

    check_run.mark_queued().await?;

    let pull = payload.pull_request;
    let installation = payload.installation;

    let job = Job {
        base: pull.base,
        head: pull.head,
        pull_request: pull.number,
        files: changed_dmis,
        check_run,
        installation: InstallationId(installation.id),
    };

    journal.lock().await.add_job(job.clone()).await;
    job_sender.0.send_async(job).await?;

    Ok(())
}

#[actix_web::post("/payload")]
pub async fn process_github_payload_actix(
    event: GithubEvent,
    payload: String,
    job_sender: DataJobSender,
    journal: DataJobJournal,
) -> actix_web::Result<&'static str> {
    // TODO: Handle reruns
    if event.0 != "pull_request" {
        return Ok("Not a pull request event");
    }

    let payload: PullRequestEventPayload = serde_json::from_str(&payload)?;

    handle_pull_request(payload, job_sender, journal)
        .await
        .map_err(actix_web::error::ErrorBadRequest)?;

    Ok("")
}
