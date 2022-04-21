use chrono;
use rocket::http::Status;
use rocket::outcome::Outcome;
use rocket::request;
use rocket::request::FromRequest;
use rocket::serde::json::serde_json;
use rocket::Request;
use rocket::State;
use serde::Deserialize;
use serde::Serialize;

use crate::github_types::*;
use crate::job;
use crate::Config;

async fn process_pull(
    pull: &PullRequest,
    run: &CheckRun,
    installation: &Installation,
    job_sender: &job::JobSender,
) {
    let repo = &pull.head.repo;
    let files: Vec<ModifiedFile> = octocrab::instance()
        .installation(installation.id.into())
        .get(
            &format!(
                "/repos/{repo}/pulls/{pull_number}/files",
                repo = repo.full_name(),
                pull_number = pull.number
            ),
            None::<&()>,
        )
        .await
        .expect("Could not get files");

    let files: Vec<ModifiedFile> = files
        .into_iter()
        .filter(|f| f.status != "removed" && f.filename.ends_with(".dmm"))
        .collect();

    if files.is_empty() {
        let _: Empty = octocrab::instance()
            .installation(installation.id.into())
            .patch(
                format!(
                    "/repos/{repo}/check-runs/{check_run_id}",
                    repo = repo.full_name(),
                    check_run_id = run.id
                ),
                Some(&UpdateCheckRun {
                    conclusion: Some("skipped".to_owned()),
                    completed_at: Some(chrono::Utc::now().to_rfc3339()),
                    status: None,
                    name: None,
                    started_at: None,
                    output: None,
                }),
            )
            .await
            .expect("Could not update check run");
        return; // Ok("No files to process");
    }

    eprintln!("{}", repo.owner());

    let _: Empty = octocrab::instance()
        .installation(installation.id.into())
        .patch(
            format!(
                "/repos/{repo}/check-runs/{check_run_id}",
                repo = repo.full_name(),
                check_run_id = run.id
            ),
            Some(&UpdateCheckRun {
                conclusion: None,
                completed_at: None,
                status: Some("queued".to_string()),
                name: None,
                started_at: Some(chrono::Utc::now().to_rfc3339()),
                output: None,
            }),
        )
        .await
        .expect("Could not update check run");

    let result = job_sender
        .0
        .send_async(job::Job {
            base: pull.base.clone(),
            head: pull.head.clone(),
            pull_request: pull.number,
            files,
            repository: pull.base.repo.clone(),
            check_run_id: run.id,
            installation_id: installation.id,
        })
        .await;
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

#[derive(Serialize)]
struct CreateCheckRun {
    pub name: String,
    pub head_sha: String,
}

#[derive(Deserialize, Debug)]
struct AssFart {}

pub async fn submit_check(full_repo: String, head_sha: String, inst_id: u64) {
    let _: AssFart = octocrab::instance()
        .installation(inst_id.into())
        .post(
            format!("/repos/{full_repo}/check-runs"),
            Some(&CreateCheckRun {
                name: "MapDiffBot2".to_string(),
                head_sha,
            }),
        )
        .await
        .expect("Could not create check run");
}

#[post("/payload", format = "json", data = "<payload>")]
pub async fn process_github_payload(
    event: GithubEvent,
    payload: String,
    job_sender: &State<job::JobSender>,
) -> Result<&'static str, &'static str> {
    match event.0.as_str() {
        "check_suite" => {
            let payload: JobPayload = serde_json::from_str(&payload).unwrap();
            println!("{:#?}", payload);
            submit_check(
                payload.repository.full_name(),
                payload.check_suite.unwrap().head_sha,
                payload.installation.id,
            )
            .await;
        }
        "check_run" => {
            let payload: JobPayload = serde_json::from_str(&payload).unwrap();
            if let Some(check_run) = payload.check_run {
                if check_run.app.id != 192759 {
                    return Ok("Not MapDiffBot2");
                }
                if payload.action == "created" {
                    process_pull(
                        &check_run.pull_requests[0],
                        &check_run,
                        &payload.installation,
                        job_sender,
                    )
                    .await;
                }
            }
        }
        _ => {
            println!("{}", event.0);
            //println!("{}", payload);
            return Ok("Not a job event");
        }
    }

    Ok("Job submitted!")
}
