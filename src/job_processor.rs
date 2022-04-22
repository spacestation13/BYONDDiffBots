use ahash::RandomState;
use dmm_tools::render_passes::RenderPass;
use flume::Receiver;
use path_absolutize::*;
use std::collections::HashSet;
use std::path::Path;
use std::process::Command;
use std::sync::RwLock;

extern crate dreammaker as dm;

use crate::git_operations::*;
use crate::github_types::{Branch, Empty, ModifiedFile, Output, Repository, UpdateCheckRun};
use crate::job::Job;
use crate::render_error::RenderError;
use crate::rendering::{get_map_diffs, render_map, Context, MapDiff};
use crate::{job, CONFIG};

fn render_befores(
    base_context: &Context,
    diffs: &Vec<MapDiff>,
    render_passes: &Vec<Box<dyn RenderPass>>,
    output_dir: &Path,
) {
    eprintln!("rendering befores");
    for (idx, diff) in diffs.iter().enumerate() {
        eprintln!("rendering before");
        let before = render_map(
            &base_context,
            &diff.base_map,
            &diff.bounding_box,
            &Default::default(),
            render_passes,
        )
        .unwrap();

        let directory = format!("{}/{}", output_dir.display(), idx);

        eprintln!("Creating output directory: {}", directory);
        std::fs::create_dir_all(&directory).expect("Could not create path");

        eprintln!("saving images");
        before
            .to_file(format!("{}/{}", directory, "before.png").as_ref())
            .unwrap();
    }
}

fn render_afters(
    head_context: &Context,
    diffs: &Vec<MapDiff>,
    render_passes: &Vec<Box<dyn RenderPass>>,
    output_dir: &Path,
) -> Result<(), RenderError> {
    eprintln!("rendering afters");
    for (idx, diff) in diffs.iter().enumerate() {
        eprintln!("rendering after");
        let after = render_map(
            &head_context,
            &diff.head_map,
            &diff.bounding_box,
            &Default::default(),
            render_passes,
        )?;
        eprintln!("rendering after");

        let directory = format!("{}/{}", output_dir.display(), idx);

        eprintln!("Creating output directory: {}", directory);
        std::fs::create_dir_all(&directory)?;

        eprintln!("saving images");
        after.to_file(format!("{}/{}", directory, "after.png").as_ref())?;
    }
    Ok(())
}

fn render(
    repo: &Repository,
    base: &Branch,
    head: &Branch,
    files: &Vec<ModifiedFile>,
    output_dir: &Path,
    pull_request_number: u64,
) -> Result<(), RenderError> {
    let errors: RwLock<HashSet<String, RandomState>> = Default::default();

    eprintln!("Parsing base");
    let mut base_context = Context::default();
    base_context.objtree(&Path::new(&repo.name).absolutize()?)?;

    let pull_branch = format!("mdb-{}-{}", base.sha, head.sha);
    let fetch_branch = format!("pull/{}/head:{}", pull_request_number, pull_branch);

    with_repo_dir(&base.repo.name, || {
        Command::new("git")
            .args(["fetch", "origin", &fetch_branch])
            .output()?;
        Ok(())
    })?;

    eprintln!("Parsing head");
    let mut head_context = Context::default();
    with_checkout(&repo.name, &fetch_branch, || {
        let p = Path::new(".").absolutize()?;
        head_context.objtree(&p)?;
        Ok(())
    })?;

    let render_passes = &dmm_tools::render_passes::configure(
        &base_context.dm_context.config().map_renderer, //TODO: also use render passes from head context
        "",
        "",
    );
    let diffs = get_map_diffs(&base, &pull_branch, files);

    render_befores(&base_context, &diffs, render_passes, output_dir);

    with_checkout(&repo.name, &pull_branch, || {
        render_afters(&head_context, &diffs, render_passes, output_dir)?;
        Ok(())
    })?;
    Ok(())
}

async fn handle_job(job: Job) -> Result<(), RenderError> {
    // Done this way rather than `let _ = ...` because `patch` needs to know the expected
    // type returned from the github api
    let _: Empty = octocrab::instance()
        .installation(job.installation_id.into())
        .patch(
            format!(
                "/repos/{repo}/check-runs/{check_run_id}",
                repo = job.repository.full_name(),
                check_run_id = job.check_run_id
            ),
            Some(&UpdateCheckRun {
                conclusion: None,
                completed_at: None,
                status: Some("in_progress".to_string()),
                name: None,
                started_at: Some(chrono::Utc::now().to_rfc3339()),
                output: None,
            }),
        )
        .await?;

    let base = job.base;
    let head = job.head;
    let repo = format!("https://github.com/{}", base.repo.full_name());
    Command::new("git")
        .args(["clone", "-C", "./repos", &repo])
        .output()?;

    let non_abs_directory = format!("images/{}/{}", job.repository.id, job.check_run_id);
    let directory = Path::new(&non_abs_directory).absolutize()?;
    let directory = directory.as_ref().to_str().ok_or(RenderError::Other(
        "Failed to create absolute path to image directory".to_owned(),
    ))?;

    git_checkout(
        &job.repository
            .default_branch
            .clone()
            .unwrap_or("master".to_owned()),
    )?;

    render(
        &base.repo,
        &base,
        &head,
        &job.files,
        Path::new(directory),
        job.pull_request,
    )?;

    let conf = CONFIG.read().await;
    let file_url = &conf.as_ref().unwrap().file_hosting_url;

    let link_before = format!("{}/{}/0/before.png", file_url, non_abs_directory);
    let link_after = format!("{}/{}/0/after.png", file_url, non_abs_directory);

    let title = "Map renderings";
    let summary = "Maps with diff:";
    let text = format!(
        "\
<details>
	<summary>
	{}
	</summary>

|  Old  |      New      |  Difference  |
| :---: |     :---:     |    :---:     |
|![]({})|    ![]({})    |coming soon...|

</details>",
        job.files[0].filename, link_before, link_after
    );

    let output = Output {
        title: title.to_owned(),
        summary: summary.to_owned(),
        text,
    };

    let _: Empty = octocrab::instance()
        .installation(job.installation_id.into())
        .patch(
            format!(
                "/repos/{repo}/check-runs/{check_run_id}",
                repo = job.repository.full_name(),
                check_run_id = job.check_run_id
            ),
            Some(&UpdateCheckRun {
                conclusion: Some("success".to_string()),
                completed_at: Some(chrono::Utc::now().to_rfc3339()),
                status: None,
                name: None,
                started_at: None,
                output: Some(output),
            }),
        )
        .await?;

    Ok(())
}

pub async fn handle_jobs(job_receiver: Receiver<job::Job>) {
    eprintln!("Starting job handler");
    while let Ok(job) = job_receiver.recv_async().await {
        eprintln!("Received job: {:#?}", job);
        if let Err(err) = handle_job(job).await {
            eprintln!("Error handling job: {:?}", err);
        }
    }
}
