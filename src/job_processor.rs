use anyhow::{Context, Result};
use flume::Receiver;
use path_absolutize::*;
use std::path::Path;
use std::process::Command;
use std::time::Instant;

extern crate dreammaker as dm;

use crate::git_operations::*;
use crate::github_api::update_check_run;
use crate::github_types::*;
use crate::job::Job;
use crate::rendering::*;
use crate::{job, CONFIG};

fn render(
    base: &Branch,
    head: &Branch,
    added_files: &[&ModifiedFile],
    modified_files: &[&ModifiedFile],
    removed_files: &[&ModifiedFile],
    output_dir: &Path,
    pull_request_number: u64,
) -> Result<()> {
    eprintln!("Parsing base");
    let now = Instant::now();
    let path = format!("./repos/{}", &base.repo.name);
    let path = Path::new(&path)
        .absolutize()
        .context("Making repo path absolute")?;
    let base_context = RenderingContext::new(&path).context("Parsing base")?;
    eprintln!("Parsing base took {}s", now.elapsed().as_secs());

    let pull_branch = format!("mdb-{}-{}", base.sha, head.sha);
    let fetch_branch = format!("pull/{}/head:{}", pull_request_number, pull_branch);

    eprintln!("Fetching and parsing head");
    let now = Instant::now();
    with_repo_dir(&base.repo.name, || {
        Command::new("git")
            .args(["fetch", "origin", &fetch_branch])
            .output()
            .context("Running fetch command")?;
        Ok(())
    })
    .context("Fetching the pull")?;

    let head_context = with_checkout(&base.repo.name, &pull_branch, || {
        RenderingContext::new(&path)
    })
    .context("Parsing head")?;

    eprintln!(
        "Fetching and parsing head took {}s",
        now.elapsed().as_secs()
    );

    let base_render_passes = &dmm_tools::render_passes::configure(
        &base_context.config().map_renderer,
        "",
        "hide-space,hide-invisible,random",
    );

    let head_render_passes = &dmm_tools::render_passes::configure(
        &head_context.config().map_renderer,
        "",
        "hide-space,hide-invisible,random",
    );

    // ADDED MAPS
    let added_directory = format!("{}/a", output_dir.display());
    let added_directory = Path::new(&added_directory);

    let added_errors = Default::default();
    let now = Instant::now();
    with_checkout(&base.repo.name, &pull_branch, || {
        let maps = load_maps(added_files).context("Loading added maps")?;
        let bounds = maps
            .iter()
            .map(|m| BoundingBox::for_full_map(m))
            .collect::<Vec<BoundingBox>>();

        render_map_regions(
            &head_context,
            &maps,
            &bounds,
            &head_render_passes,
            &added_directory,
            "added.png",
            &added_errors,
        )
        .context("Rendering added maps")
    })?;
    eprintln!("Added maps took {}s", now.elapsed().as_secs());

    // MODIFIED MAPS
    let modified_directory = format!("{}/m", output_dir.display());
    let modified_directory = Path::new(&modified_directory);

    let base_maps = with_repo_dir(&base.repo.name, || load_maps(modified_files))
        .context("Loading base maps")?;
    let head_maps = with_checkout(&base.repo.name, &pull_branch, || load_maps(modified_files))
        .context("Loading head maps")?;
    let diff_bounds = get_map_diff_bounding_boxes(&base_maps, &head_maps);

    let modified_before_errors = Default::default();
    let now = Instant::now();

    render_map_regions(
        &base_context,
        &base_maps,
        &diff_bounds,
        &head_render_passes,
        &modified_directory,
        "before.png",
        &modified_before_errors,
    )
    .context("Rendering modified before maps")?;

    eprintln!("Modified before maps took {}s", now.elapsed().as_secs());

    let modified_after_errors = Default::default();
    let now = Instant::now();
    with_checkout(&base.repo.name, &pull_branch, || {
        render_map_regions(
            &head_context,
            &head_maps,
            &diff_bounds,
            head_render_passes,
            modified_directory,
            "after.png",
            &modified_after_errors,
        )?;
        Ok(())
    })
    .context("Rendering modified after maps")?;
    eprintln!("Modified after maps took {}s", now.elapsed().as_secs());

    // REMOVED MAPS
    let removed_directory = format!("{}/r", output_dir.display());
    let removed_directory = Path::new(&removed_directory);

    let removed_errors = Default::default();
    let now = Instant::now();
    with_repo_dir(&base.repo.name, || {
        let maps = load_maps(removed_files).context("Loading removed maps")?;
        let bounds = maps
            .iter()
            .map(|m| BoundingBox::for_full_map(m))
            .collect::<Vec<BoundingBox>>();

        render_map_regions(
            &base_context,
            &maps,
            &bounds,
            &base_render_passes,
            &removed_directory,
            "removed.png",
            &removed_errors,
        )
        .context("Rendering removed maps")
    })?;
    eprintln!("Removed maps took {}s", now.elapsed().as_secs());

    with_repo_dir(&base.repo.name, || {
        Command::new("git")
            .args(["branch", "-d", &pull_branch])
            .output()?;
        Ok(())
    })
    .context("Deleting pull branch")?;

    let print_errors = |e: &RenderingErrors| {
        for error in e.read().unwrap().iter() {
            eprintln!("{}", error);
        }
    };

    eprintln!("Added map errors: ");
    print_errors(&added_errors);
    eprintln!("Modified before map errors: ");
    print_errors(&modified_before_errors);
    eprintln!("Modified after map errors: ");
    print_errors(&modified_after_errors);
    eprintln!("Removed map errors: ");
    print_errors(&removed_errors);

    Ok(())
}

async fn handle_job(job: Job) -> Result<()> {
    update_check_run(
        &job,
        UpdateCheckRunBuilder::default()
            .status("in_progress")
            .started_at(chrono::Utc::now().to_rfc3339()),
    )
    .await?;

    let base = &job.base;
    let head = &job.head;
    let repo = format!("https://github.com/{}", base.repo.full_name());
    Command::new("git")
        .args(["clone", &repo, &format!("./repos/{}", base.repo.name)])
        .output()
        .context("Cloning repo")?;

    let non_abs_directory = format!("images/{}/{}", job.base.repo.id, job.check_run_id);
    let directory = Path::new(&non_abs_directory)
        .absolutize()
        .context("Absolutizing images path")?;
    let directory = directory
        .as_ref()
        .to_str()
        .ok_or_else(|| anyhow::anyhow!("Failed to create absolute path to image directory",))?;

    git_checkout(
        &job.base
            .repo
            .default_branch
            .clone()
            .unwrap_or_else(|| "master".to_owned()),
    )
    .context("Checking out to default branch")?; // If this fails, good luck

    let filter_on_status = |status: &str| {
        job.files
            .iter()
            .filter(|f| f.status == status)
            .collect::<Vec<&ModifiedFile>>()
    };

    let added_files = filter_on_status("added");
    let modified_files = filter_on_status("modified");
    let removed_files = filter_on_status("removed");

    render(
        base,
        head,
        &added_files,
        &modified_files,
        &removed_files,
        Path::new(directory),
        job.pull_request,
    )
    .context("Doing the renderance")?;

    let conf = CONFIG.read().await;
    let file_url = &conf.as_ref().unwrap().file_hosting_url;

    let title = "Map renderings";
    let summary = "Maps with diff:";
    let mut text = String::new();

    for (idx, file) in added_files.iter().enumerate() {
        let link = format!("{}/{}/a/{}/added.png", file_url, non_abs_directory, idx);
        text.push_str(&format!(
            include_str!("diff_template_add.txt"),
            filename = file.filename,
            image_link = link
        ));
    }

    for (idx, file) in modified_files.iter().enumerate() {
        let link_before = format!("{}/{}/m/{}/before.png", file_url, non_abs_directory, idx);
        let link_after = format!("{}/{}/m/{}/after.png", file_url, non_abs_directory, idx);
        text.push_str(&format!(
            include_str!("diff_template_mod.txt"),
            filename = file.filename,
            image_before_link = link_before,
            image_after_link = link_after
        ));
    }

    for (idx, file) in removed_files.iter().enumerate() {
        let link = format!("{}/{}/r/{}/removed.png", file_url, non_abs_directory, idx);
        text.push_str(&format!(
            include_str!("diff_template_remove.txt"),
            filename = file.filename,
            image_link = link
        ));
    }

    let output = Output {
        title: title.to_owned(),
        summary: summary.to_owned(),
        text,
    };

    update_check_run(
        &job,
        UpdateCheckRunBuilder::default()
            .conclusion("success")
            .completed_at(chrono::Utc::now().to_rfc3339())
            .output(output),
    )
    .await
    .context("Updating check run with success")?;

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
