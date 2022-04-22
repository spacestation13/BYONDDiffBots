use ahash::RandomState;
use anyhow::Result;
use dmm_tools::dmm;
use dmm_tools::render_passes::RenderPass;
use flume::Receiver;
use path_absolutize::*;
use rayon::prelude::*;
use std::collections::HashSet;
use std::path::Path;
use std::process::Command;
use std::sync::RwLock;
use std::time::{Duration, Instant};

extern crate dreammaker as dm;

use crate::git_operations::*;
use crate::github_types::{Branch, Empty, ModifiedFile, Output, Repository, UpdateCheckRun};
use crate::job::Job;
use crate::rendering::{get_map_diffs, render_map, BoundingBox, Context, MapDiffs};
use crate::{job, CONFIG};

fn do_render(
    context: &Context,
    maps: &Vec<dmm::Map>,
    bbs: &Vec<BoundingBox>,
    render_passes: &Vec<Box<dyn RenderPass>>,
    output_dir: &Path,
    filename: &str,
) -> Result<()> {
    let objtree = &context.objtree;
    let icon_cache = &context.icon_cache;
    let _: Result<()> = maps
        .par_iter()
        .zip(bbs.par_iter())
        .enumerate()
        .map(|(idx, (map, bb))| {
            eprintln!("rendering map {}", idx);
            let image = render_map(
                objtree,
                icon_cache,
                map,
                bb,
                &Default::default(),
                render_passes,
            )?;

            let directory = format!("{}/{}", output_dir.display(), idx);

            eprintln!("Creating output directory: {}", directory);
            std::fs::create_dir_all(&directory)?;

            eprintln!("saving images");
            image.to_file(format!("{}/{}", directory, filename).as_ref())?;
            Ok(())
        })
        .collect();
    Ok(())
}

fn render_befores(
    base_context: &Context,
    maps: &Vec<dmm::Map>,
    bbs: &Vec<BoundingBox>,
    render_passes: &Vec<Box<dyn RenderPass>>,
    output_dir: &Path,
) -> Result<()> {
    eprintln!("Rendering befores");
    do_render(
        base_context,
        maps,
        bbs,
        render_passes,
        output_dir,
        "before.png",
    )
}

fn render_afters(
    head_context: &Context,
    maps: &Vec<dmm::Map>,
    bbs: &Vec<BoundingBox>,
    render_passes: &Vec<Box<dyn RenderPass>>,
    output_dir: &Path,
) -> Result<()> {
    eprintln!("Rendering afters");
    do_render(
        head_context,
        maps,
        bbs,
        render_passes,
        output_dir,
        "after.png",
    )
}

fn render(
    repo: &Repository,
    base: &Branch,
    head: &Branch,
    files: &Vec<ModifiedFile>,
    output_dir: &Path,
    pull_request_number: u64,
) -> Result<()> {
    let errors: RwLock<HashSet<String, RandomState>> = Default::default();

    eprintln!("Parsing base");
    let now = Instant::now();
    let mut base_context = Context::default();
    with_repo_dir(&repo.name, || {
        let p = Path::new(".").absolutize()?;
        base_context.objtree(&p)?;
        Ok(())
    })?;
    eprintln!("Parsing base took {}s", now.elapsed().as_secs());

    let pull_branch = format!("mdb-{}-{}", base.sha, head.sha);
    let fetch_branch = format!("pull/{}/head:{}", pull_request_number, pull_branch);

    eprintln!("Fetching and parsing head");
    let now = Instant::now();
    with_repo_dir(&base.repo.name, || {
        Command::new("git")
            .args(["fetch", "origin", &fetch_branch])
            .output()?;
        Ok(())
    })?;

    let mut head_context = Context::default();
    with_checkout(&repo.name, &fetch_branch, || {
        let p = Path::new(".").absolutize()?;
        head_context.objtree(&p)?;
        Ok(())
    })?;
    eprintln!(
        "Fetching and parsing head took {}s",
        now.elapsed().as_secs()
    );

    let render_passes = &dmm_tools::render_passes::configure(
        &base_context.dm_context.config().map_renderer, //TODO: also use render passes from head context
        "",
        "",
    );
    let diffs = get_map_diffs(&base, &pull_branch, files)?;

    let now = Instant::now();
    render_befores(
        &base_context,
        &diffs.bases,
        &diffs.bbs,
        render_passes,
        output_dir,
    )?;
    eprintln!("rendering befores took {}s", now.elapsed().as_secs());

    let now = Instant::now();
    with_checkout(&repo.name, &pull_branch, || {
        render_afters(
            &head_context,
            &diffs.heads,
            &diffs.bbs,
            render_passes,
            output_dir,
        )?;
        Ok(())
    })?;
    eprintln!("rendering afters took {}s", now.elapsed().as_secs());
    Ok(())
}

async fn handle_job(job: Job) -> Result<()> {
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
    let output = Command::new("git")
        .args(["clone", &repo, &format!("./repos/{}", base.repo.name)])
        .output()?;

    println!("{}", String::from_utf8_lossy(&output.stdout));
    println!("{}", String::from_utf8_lossy(&output.stderr));

    let non_abs_directory = format!("images/{}/{}", job.repository.id, job.check_run_id);
    let directory = Path::new(&non_abs_directory).absolutize()?;
    let directory = directory.as_ref().to_str().ok_or(anyhow::anyhow!(
        "Failed to create absolute path to image directory",
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

    let title = "Map renderings";
    let summary = "Maps with diff:";
    let mut text = String::new();

    for (idx, file) in job.files.iter().enumerate() {
        let link_before = format!("{}/{}/{}/before.png", file_url, non_abs_directory, idx);
        let link_after = format!("{}/{}/{}/after.png", file_url, non_abs_directory, idx);
        text.push_str(&format!(
            include_str!("diff_template.txt"),
            file.filename, link_before, link_after
        ));
    }

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
        let now = Instant::now();
        if let Err(err) = handle_job(job).await {
            eprintln!("Error handling job: {:?}", err);
        } else {
            eprintln!("Job handled successfully");
        }
        eprintln!("Handling job took {}s", now.elapsed().as_secs());
    }
}
