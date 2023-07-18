use diffbot_lib::log;
use eyre::{Context, Result};
use mysql_async::{params, prelude::Queryable};
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
    log::debug!("Processing pull request");

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

    let files = match get_pull_files(repo.name_tuple(), installation.id, &pull)
        .await
        .context("Getting files modified by PR")
    {
        Ok(files) => files
            .into_iter()
            .filter(|f| f.filename.ends_with(".dmm"))
            .filter(|f| {
                matches!(
                    f.status,
                    ChangeType::Added | ChangeType::Deleted | ChangeType::Modified
                )
            })
            .collect::<Vec<_>>(),
        Err(err) => {
            check_run.mark_failed(&format!("{:?}", err)).await?;
            return Ok(());
        }
    };

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

    let job = serde_json::to_vec(&JobType::GithubJob(Box::new(job)))?;

    job_sender.lock().await.send(job).await?;

    log::debug!("Job sent to queue");

    Ok(())
}

async fn handle_pull_request(
    payload: String,
    job_sender: DataJobSender,
    pool: actix_web::web::Data<Option<mysql_async::Pool>>,
) -> Result<&'static str> {
    let payload: PullRequestEventPayload = serde_json::from_str(&payload)?;

    let pool = pool.get_ref();

    match payload.action.as_str() {
        "opened" | "synchronize" => {
            log::debug!("Creating checkrun");

            let check_run = CheckRun::create(
                &payload.repository.full_name(),
                &payload.pull_request.head.sha,
                payload.installation.id,
                Some("MapDiffBot2"),
            )
            .await?;

            let (check_id, repo_id, pr_number) = (
                check_run.id(),
                payload.repository.id,
                payload.pull_request.number,
            );

            process_pull(
                payload.repository,
                payload.pull_request,
                check_run,
                &payload.installation,
                job_sender,
            )
            .await?;

            if let Some(ref pool) = pool {
                let mut conn = match pool.get_conn().await {
                    Ok(conn) => conn,
                    Err(e) => {
                        log::error!("{:?}", e);
                        return Ok("Getting mysql connection failed");
                    }
                };
                if let Err(e) = conn
                    .exec_drop(
                        r"INSERT INTO jobs (
                        check_id,
                        repo_id,
                        pr_number,
                        merge_date
                    )
                    VALUES(
                        :check_id,
                        :repo_id,
                        :pr_number,
                        :merge_date
                    )
                    ",
                        params! {
                            "check_id" => check_id,
                            "repo_id" => repo_id,
                            "pr_number" => pr_number,
                            "merge_date" => None::<time::PrimitiveDateTime>,
                        },
                    )
                    .await
                {
                    log::error!("{:?}", e);
                };
            }
        }
        "closed" => {
            if let Some(ref pool) = pool {
                let mut conn = match pool.get_conn().await {
                    Ok(conn) => conn,
                    Err(e) => {
                        log::error!("{:?}", e);
                        return Ok("Getting mysql connection failed");
                    }
                };

                let now = time::OffsetDateTime::now_utc();
                let now = time::PrimitiveDateTime::new(now.date(), now.time());
                if let Err(e) = conn
                    .exec_drop(
                        r"UPDATE jobs SET merge_date=:date
                    WHERE repo_id=:rp_id
                    AND pr_number=:pr_num",
                        params! {
                            "date" => now,
                            "rp_id" => payload.repository.id,
                            "pr_num" => payload.pull_request.number,
                        },
                    )
                    .await
                {
                    log::error!("{:?}", e);
                };
            }
        }
        _ => return Ok("PR not opened or updated"),
    }

    Ok("Check submitted")
}

#[actix_web::post("/payload")]
pub async fn process_github_payload(
    event: diffbot_lib::github::github_api::GithubEvent,
    payload: String,
    job_sender: DataJobSender,
    pool: actix_web::web::Data<Option<mysql_async::Pool>>,
) -> actix_web::Result<&'static str> {
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

    log::debug!("Payload received, processing");

    handle_pull_request(payload, job_sender, pool)
        .await
        .map_err(|e| {
            log::error!("Error handling event: {:?}", e);
            actix_web::error::ErrorBadRequest(e)
        })
}
