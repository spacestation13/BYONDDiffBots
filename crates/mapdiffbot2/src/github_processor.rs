use std::sync::Arc;

use anyhow::{Context, Result};
use rocket::http::Status;
use rocket::outcome::Outcome;
use rocket::request;
use rocket::request::FromRequest;
use rocket::tokio::sync::Mutex;
use rocket::Request;
use rocket::State;

use crate::CONFIG;
use diffbot_lib::github::github_api::*;
use diffbot_lib::github::github_types::*;
use diffbot_lib::job::types::{JobSender, JobJournal, Job};

async fn process_pull(
    pull: PullRequest,
    check_run: CheckRun,
    installation: &Installation,
    job_sender: &JobSender,
    journal: &Arc<Mutex<JobJournal>>,
) -> Result<()> {
    if pull
        .title
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("PR title is None"))?
        .to_ascii_lowercase()
        .contains("[mdb ignore]")
    {
        let output = Output {
            title: "PR Ignored".to_owned(),
            summary: "This PR has `[MDB IGNORE]` in the title. Aborting.".to_owned(),
            text: "".to_owned(),
        };

        check_run.mark_skipped(output).await?;

        return Ok(());
    }

    let (blacklist, contact) = {
        let conf = &CONFIG.get().unwrap();
        (&conf.blacklist, &conf.blacklist_contact)
    };

    if blacklist.contains(&pull.base.repo.id) {
        let output = Output {
            title: "Repo blacklisted".to_owned(),
            summary: format!(
                "Repository {} is blacklisted. {}",
                pull.base.repo.full_name(),
                contact
            ),
            text: "".to_owned(),
        };

        check_run.mark_skipped(output).await?;

        return Ok(());
    }

    let files: Vec<ModifiedFile> = get_pull_files(installation, &pull)
        .await
        .context("Getting files modified by PR")?
        .into_iter()
        .filter(|f| f.filename.ends_with(".dmm"))
        .collect();

    if files.is_empty() {
        let output = Output {
            title: "No map changes".to_owned(),
            summary: "There are no changed map files to render.".to_owned(),
            text: "".to_owned(),
        };

        check_run.mark_skipped(output).await?;

        return Ok(());
    }

    check_run.mark_queued().await?;

    let job = Job {
        base: pull.base,
        head: pull.head,
        pull_request: pull.number,
        files,
        check_run,
    };

    journal.lock().await.add_job(job.clone()).await;
    job_sender.0.send_async(job).await?;

    Ok(())
}

#[derive(Debug)]
pub struct GithubEvent(pub String);

#[rocket::async_trait]
impl<'r> FromRequest<'r> for GithubEvent {
    type Error = &'static str;

    async fn from_request(req: &'r Request<'_>) -> request::Outcome<Self, Self::Error> {
        match req.headers().get_one("X-Github-Event") {
            Some(event) => Outcome::Success(GithubEvent(event.to_owned())),
            None => Outcome::Failure((Status::BadRequest, "Missing X-Github-Event header")),
        }
    }
}

async fn handle_pull_request(
    payload: String,
    job_sender: &State<JobSender>,
    journal: &State<Arc<Mutex<JobJournal>>>,
) -> Result<&'static str> {
    let payload: PullRequestEventPayload = serde_json::from_str(&payload)?;
    if payload.action != "opened" && payload.action != "synchronize" {
        return Ok("PR not opened or updated");
    }

    let check_run = CheckRun::create(
        &payload.pull_request.base.repo.full_name(),
        &payload.pull_request.head.sha,
        payload.installation.id,
        None,
    )
    .await?;

    process_pull(
        payload.pull_request,
        check_run,
        &payload.installation,
        job_sender,
        journal,
    )
    .await?;

    Ok("Check submitted")
}

#[post("/payload", format = "json", data = "<payload>")]
pub async fn process_github_payload(
    event: GithubEvent,
    payload: String,
    job_sender: &State<JobSender>,
    journal: &State<Arc<Mutex<JobJournal>>>,
) -> Result<&'static str, &'static str> {
    if event.0 != "pull_request" {
        return Ok("Not a pull request event");
    }

    handle_pull_request(payload, job_sender, journal)
        .await
        .map_err(|e| {
            eprintln!("Error handling event: {:?}", e);
            "An error occured while handling the event"
        })
}
