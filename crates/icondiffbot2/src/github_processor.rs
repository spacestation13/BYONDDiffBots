use diffbot_lib::{
    github::{
        github_api::CheckRun,
        github_types::{ChangeType, Output, PullRequestEventPayload},
        graphql::get_pull_files,
    },
    job::types::Job,
};
use eyre::Result;
use octocrab::models::InstallationId;

use diffbot_lib::github::github_types::FileDiff;

use crate::DataJobSender;

async fn handle_pull_request(
    payload: PullRequestEventPayload,
    job_sender: DataJobSender,
) -> Result<()> {
    match payload.action.as_str() {
        "opened" => {}
        #[cfg(debug_assertions)]
        "reopened" => {}
        "synchronize" => {}
        _ => return Ok(()),
    }

    let check_run = CheckRun::create(
        &payload.repository.full_name(),
        &payload.pull_request.head.sha,
        payload.installation.id,
        Some("IconDiffBot2"),
    )
    .await?;

    if payload
        .pull_request
        .title
        .as_ref()
        .ok_or_else(|| eyre::anyhow!("PR title is None"))?
        .to_ascii_lowercase()
        .contains("[idb ignore]")
    {
        let output = Output {
            title: "PR Ignored",
            summary: "This PR has `[IDB IGNORE]` in the title. Aborting.".to_owned(),
            text: "".to_owned(),
        };

        check_run.mark_skipped(output).await?;
        return Ok(());
    }

    let conf = &crate::CONFIG.get().unwrap();
    let (blacklist, contact) = (&conf.blacklist, &conf.blacklist_contact);

    if blacklist.contains(&payload.repository.id) {
        let output = Output {
            title: "Repo blacklisted",
            summary: format!(
                "Repository {} is blacklisted. {}",
                payload.repository.full_name(),
                contact
            ),
            text: "".to_owned(),
        };

        check_run.mark_skipped(output).await?;

        return Ok(());
    }

    let files = get_pull_files(
        payload.repository.name_tuple(),
        payload.installation.id,
        &payload.pull_request,
    )
    .await?;

    let changed_dmis: Vec<FileDiff> = files
        .into_iter()
        .filter(|e| e.filename.ends_with(".dmi"))
        .filter(|e| {
            matches!(
                e.status,
                ChangeType::Added | ChangeType::Deleted | ChangeType::Modified
            )
        })
        .collect();

    if changed_dmis.is_empty() {
        let output = Output {
            title: "No icon changes",
            summary: "There are no relevant changed icon files to render.".to_owned(),
            text: "".to_owned(),
        };

        check_run.mark_skipped(output).await?;

        return Ok(());
    }

    check_run.mark_queued().await?;

    let pull = payload.pull_request;
    let installation = payload.installation;

    let job = Job {
        repo: payload.repository,
        base: pull.base,
        head: pull.head,
        pull_request: pull.number,
        files: changed_dmis,
        check_run,
        installation: InstallationId(installation.id),
    };

    let job = serde_json::to_vec(&job)?;

    job_sender.lock().await.send(job).await?;

    Ok(())
}

#[actix_web::post("/payload")]
pub async fn process_github_payload_actix(
    event: diffbot_lib::github::github_api::GithubEvent,
    payload: String,
    job_sender: DataJobSender,
) -> actix_web::Result<&'static str> {
    // TODO: Handle reruns
    if event.0 != "pull_request" {
        return Ok("Not a pull request event");
    }

    let secret = {
        let conf = &crate::CONFIG.get().unwrap();
        conf.secret.as_ref()
    };

    diffbot_lib::verify::verify_signature(
        secret.map(|a| a.as_str()),
        event.1.as_deref(),
        &payload,
    )?;

    let payload: PullRequestEventPayload = serde_json::from_str(&payload)?;

    handle_pull_request(payload, job_sender)
        .await
        .map_err(actix_web::error::ErrorBadRequest)?;

    Ok("")
}
