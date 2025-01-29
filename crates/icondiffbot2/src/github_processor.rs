use diffbot_lib::{
    github::{
        github_api::CheckRun,
        github_types::{ChangeType, FileDiff, Output, PullRequestEventPayload},
        graphql::get_pull_files,
    },
    job::types::Job,
    tracing,
};
use eyre::Result;
use octocrab::models::InstallationId;

use mysql_async::{params, prelude::Queryable};

use crate::DataJobSender;

async fn handle_pull_request(
    payload: PullRequestEventPayload,
    job_sender: DataJobSender,
    pool: actix_web::web::Data<Option<mysql_async::Pool>>,
) -> Result<()> {
    let pool = pool.get_ref();

    match payload.action.as_str() {
        "opened" | "synchronize" => {
            let check_run = CheckRun::create(
                &payload.repository.full_name(),
                &payload.pull_request.head.sha,
                payload.installation.id,
                Some("IconDiffBot2"),
            )
            .await?;

            let (check_id, repo_id, pr_number) = (
                check_run.id(),
                payload.repository.id,
                payload.pull_request.number,
            );

            let num_icons = handle_pull(payload, job_sender, check_run).await?;

            if let Some(ref pool) = pool {
                let mut conn = match pool.get_conn().await {
                    Ok(conn) => conn,
                    Err(e) => {
                        tracing::error!("{:?}", e);
                        return Ok(());
                    }
                };

                if let Err(e) = conn
                    .exec_drop(
                        r"INSERT INTO jobs (
                        check_id,
                        repo_id,
                        pr_number,
                        merge_date,
                        num_icons,
                    )
                    VALUES(
                        :check_id,
                        :repo_id,
                        :pr_number,
                        :merge_date,
                        :num_icons
                    )
                    ",
                        params! {
                            "check_id" => check_id,
                            "repo_id" => repo_id,
                            "pr_number" => pr_number,
                            "merge_date" => None::<usize>,
                            "num_icons" => num_icons,
                        },
                    )
                    .await
                {
                    tracing::error!("{:?}", e);
                };
            }
            Ok(())
        }
        "closed" => {
            if let Some(ref pool) = pool {
                let mut conn = match pool.get_conn().await {
                    Ok(conn) => conn,
                    Err(e) => {
                        tracing::error!("{:?}", e);
                        return Ok(());
                    }
                };

                let now = time::OffsetDateTime::now_utc();
                if let Err(e) = conn
                    .exec_drop(
                        r"UPDATE jobs SET merge_date=:date
                    WHERE repo_id=:rp_id
                    AND pr_number=:pr_num",
                        params! {
                            "date" => mysql_async::Value::Date(now.year() as u16, now.month() as u8, now.day(), now.hour(), now.minute(), now.second(), now.microsecond()),
                            "rp_id" => payload.repository.id,
                            "pr_num" => payload.pull_request.number,
                        },
                    )
                    .await
                {
                    tracing::error!("{:?}", e);
                };
            };
            Ok(())
        }
        _ => Ok(()),
    }
}

async fn handle_pull(
    payload: PullRequestEventPayload,
    job_sender: DataJobSender,
    check_run: CheckRun,
) -> Result<usize> {
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
        return Ok(0);
    }

    let conf = &crate::CONFIG.get().unwrap();
    let (blacklist, contact) = (&conf.blacklist, &conf.blacklist_contact);

    if blacklist.contains(&payload.repository.id) {
        let output = Output {
            title: "Repo blacklisted",
            summary: format!(
                "Repository {} is blacklisted. {contact}",
                payload.repository.full_name(),
            ),
            text: "".to_owned(),
        };

        check_run.mark_skipped(output).await?;

        return Ok(0);
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

    let num_icons_diffed = changed_dmis.len();

    if changed_dmis.is_empty() {
        let output = Output {
            title: "No icon changes",
            summary: "There are no relevant changed icon files to render.".to_owned(),
            text: "".to_owned(),
        };

        check_run.mark_skipped(output).await?;

        return Ok(0);
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

    job_sender.send_async(job).await?;

    Ok(num_icons_diffed)
}

#[actix_web::post("/payload")]
pub async fn process_github_payload_actix(
    event: diffbot_lib::github::github_api::GithubEvent,
    payload: String,
    job_sender: DataJobSender,
    pool: actix_web::web::Data<Option<mysql_async::Pool>>,
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
        secret.map(|v| v.as_str()),
        event.1.as_deref(),
        &payload,
    )?;

    let payload: PullRequestEventPayload = serde_json::from_str(&payload)?;

    handle_pull_request(payload, job_sender, pool)
        .await
        .map_err(actix_web::error::ErrorBadRequest)?;

    Ok("")
}
