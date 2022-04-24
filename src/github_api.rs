use crate::{github_types::*, job::Job};
use anyhow::{Context, Result};

pub async fn update_check_run(job: &Job, builder: UpdateCheckRunBuilder) -> Result<()> {
    let check_run = builder.build().context("Building UpdateCheckRun")?;

    let _: Empty = octocrab::instance()
        .installation(job.installation_id.into())
        .patch(
            format!(
                "/repos/{repo}/check-runs/{check_run_id}",
                repo = job.base.repo.full_name(),
                check_run_id = job.check_run_id
            ),
            Some(&check_run),
        )
        .await
        .context("Getting files")?;

    Ok(())
}

pub async fn get_pull_files(
    installation: &Installation,
    pull: &PullRequest,
) -> Result<Vec<ModifiedFile>> {
    let res = octocrab::instance()
        .installation(installation.id.into())
        .get(
            &format!(
                "/repos/{repo}/pulls/{pull_number}/files",
                repo = pull.base.repo.full_name(),
                pull_number = pull.number
            ),
            None::<&()>,
        )
        .await?;

    Ok(res)
}

pub async fn submit_check(full_repo: String, head_sha: String, inst_id: u64) -> Result<()> {
    let _: Empty = octocrab::instance()
        .installation(inst_id.into())
        .post(
            format!("/repos/{full_repo}/check-runs"),
            Some(&CreateCheckRun {
                name: "MapDiffBot2".to_string(),
                head_sha,
            }),
        )
        .await
        .context("Submitting check")?;

    Ok(())
}
