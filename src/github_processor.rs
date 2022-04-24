use anyhow::{Context, Result};
use rocket::http::Status;
use rocket::outcome::Outcome;
use rocket::request;
use rocket::request::FromRequest;
use rocket::serde::json::serde_json;
use rocket::Request;
use rocket::State;

use crate::github_api::*;
use crate::github_types::*;
use crate::job::*;

async fn process_pull(
    pull: PullRequest,
    run_id: u64,
    installation: &Installation,
    job_sender: &JobSender,
) -> Result<()> {
    let files: Vec<ModifiedFile> = get_pull_files(installation, &pull)
        .await
        .context("Getting files modified by PR")?
        .into_iter()
        .filter(|f| f.filename.ends_with(".dmm"))
        .collect();

    let job = Job {
        base: pull.base,
        head: pull.head,
        pull_request: pull.number,
        files,
        check_run_id: run_id,
        installation_id: installation.id,
    };

    if job.files.is_empty() {
        update_check_run(
            &job,
            UpdateCheckRunBuilder::default()
                .status("skipped")
                .completed_at(chrono::Utc::now().to_rfc3339()),
        )
        .await
        .context("Marking check run as skipped")?;

        return Ok(());
    }

    update_check_run(
        &job,
        UpdateCheckRunBuilder::default()
            .status("queued")
            .started_at(chrono::Utc::now().to_rfc3339()),
    )
    .await
    .context("Marking check run as queued")?;

    job_sender.0.send_async(job).await?;

    Ok(())
}

#[derive(Debug)]
pub struct GithubEvent(pub String);

#[rocket::async_trait]
impl<'r> FromRequest<'r> for GithubEvent {
    type Error = &'static str;

    async fn from_request(req: &'r Request<'_>) -> request::Outcome<Self, Self::Error> {
        let event = req.headers().get_one("X-Github-Event");
        if event.is_none() {
            return Outcome::Failure((Status::BadRequest, "Missing X-Github-Event header"));
        }
        let event = GithubEvent(event.unwrap().to_string());
        Outcome::Success(event)
    }
}

#[post("/payload", format = "json", data = "<payload>")]
pub async fn process_github_payload(
    event: GithubEvent,
    payload: String,
    job_sender: &State<JobSender>,
) -> Result<&'static str, &'static str> {
    match event.0.as_str() {
        "check_suite" => {
            eprintln!("Received check_suite event");
            let payload: JobPayload = serde_json::from_str(&payload).unwrap();
            if let Err(e) = submit_check(
                payload.repository.full_name(),
                payload.check_suite.unwrap().head_sha,
                payload.installation.id,
            )
            .await
            {
                eprintln!("Failed to submit check: {:?}", e);
            }
        }
        "check_run" => {
            let payload: JobPayload = serde_json::from_str(&payload).unwrap();
            if let Some(check_run) = payload.check_run {
                if check_run.app.id != 192759 {
                    return Ok("Not MapDiffBot2");
                }
                if payload.action == "created" {
                    let pulls = check_run.pull_requests;
                    let run_id = check_run.id;
                    for pull in pulls {
                        if let Err(e) =
                            process_pull(pull, run_id, &payload.installation, job_sender).await
                        {
                            eprintln!("Failed to process pull request: {:?}", e);
                        }
                    }
                }
            }
        }
        _ => {
            return Ok("Not a job event");
        }
    }

    Ok("Job submitted!")
}
