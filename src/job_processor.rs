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
use std::time::Instant;

extern crate dreammaker as dm;

use crate::git_operations::*;
use crate::github_api::update_check_run;
use crate::github_types::*;
use crate::job::Job;
use crate::rendering::{get_map_diffs, render_map, BoundingBox, Context};
use crate::{job, CONFIG};

type RenderingErrors = RwLock<HashSet<String, RandomState>>;

fn do_render(
    context: &Context,
    maps: &Vec<dmm::Map>,
    bbs: &Vec<BoundingBox>,
    render_passes: &Vec<Box<dyn RenderPass>>,
    output_dir: &Path,
    filename: &str,
    errors: &RenderingErrors,
) -> Result<()> {
    let objtree = &context.objtree;
    let icon_cache = &context.icon_cache;
    let _: Result<()> = maps
        .par_iter()
        .zip(bbs.par_iter())
        .enumerate()
        .map(|(idx, (map, bb))| {
            eprintln!("rendering map {}", idx);
            let image = render_map(objtree, icon_cache, map, bb, errors, render_passes)?;

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
    errors: &RenderingErrors,
) -> Result<()> {
    eprintln!("Rendering befores");
    do_render(
        base_context,
        maps,
        bbs,
        render_passes,
        output_dir,
        "before.png",
        errors,
    )
}

fn render_afters(
    head_context: &Context,
    maps: &Vec<dmm::Map>,
    bbs: &Vec<BoundingBox>,
    render_passes: &Vec<Box<dyn RenderPass>>,
    output_dir: &Path,
    errors: &RenderingErrors,
) -> Result<()> {
    eprintln!("Rendering afters");
    do_render(
        head_context,
        maps,
        bbs,
        render_passes,
        output_dir,
        "after.png",
        errors,
    )
}

fn render(
    base: &Branch,
    head: &Branch,
    added_files: &Vec<&ModifiedFile>,
    modified_files: &Vec<&ModifiedFile>,
    removed_files: &Vec<&ModifiedFile>,
    output_dir: &Path,
    pull_request_number: u64,
) -> Result<()> {
    let errors: RenderingErrors = Default::default();

    eprintln!("Parsing base");
    let now = Instant::now();
    let mut base_context = Context::default();
    with_repo_dir(&base.repo.name, || {
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
    with_checkout(&base.repo.name, &pull_branch, || {
        let p = Path::new(".").absolutize()?;
        head_context.objtree(&p)?;
        Ok(())
    })?;
    eprintln!(
        "Fetching and parsing head took {}s",
        now.elapsed().as_secs()
    );

    let base_render_passes = &dmm_tools::render_passes::configure(
        &base_context.dm_context.config().map_renderer,
        "",
        "hide-space,hide-invisible,random",
    );

    let head_render_passes = &dmm_tools::render_passes::configure(
        &head_context.dm_context.config().map_renderer,
        "",
        "hide-space,hide-invisible,random",
    );

    // ADDED MAPS
    let added_directory = format!("{}/a", output_dir.display());
    let added_directory = Path::new(&added_directory);
    with_checkout(&base.repo.name, &pull_branch, || {
        let mut maps = vec![];
        let mut bbs = vec![];
        for file in added_files {
            println!("{}", file.filename);
            let map =
                dmm::Map::from_file(Path::new(&file.filename)).map_err(|e| anyhow::anyhow!(e))?;
            let size = map.dim_xyz();
            let bb = BoundingBox {
                left: 0,
                bottom: 0,
                top: size.1 - 1,
                right: size.0 - 1,
            };
            maps.push(map);
            bbs.push(bb);
        }
        do_render(
            &head_context,
            &maps,
            &bbs,
            head_render_passes,
            added_directory,
            "added.png",
            &errors,
        )
    })?;

    // MODIFIED MAPS
    let modified_directory = format!("{}/m", output_dir.display());
    let modified_directory = Path::new(&modified_directory);
    let diffs = get_map_diffs(&base, &pull_branch, &modified_files)?;

    let now = Instant::now();
    render_befores(
        &base_context,
        &diffs.bases,
        &diffs.bbs,
        base_render_passes,
        modified_directory,
        &errors,
    )?;
    eprintln!("rendering befores took {}s", now.elapsed().as_secs());

    let now = Instant::now();
    with_checkout(&base.repo.name, &pull_branch, || {
        render_afters(
            &head_context,
            &diffs.heads,
            &diffs.bbs,
            head_render_passes,
            modified_directory,
            &errors,
        )?;
        Ok(())
    })?;
    eprintln!("rendering afters took {}s", now.elapsed().as_secs());

    // REMOVED MAPS
    let removed_directory = format!("{}/r", output_dir.display());
    let removed_directory = Path::new(&removed_directory);

    let mut maps = vec![];
    let mut bbs = vec![];
    with_repo_dir(&base.repo.name, || {
        for file in removed_files {
            println!("{}", file.filename);
            let map =
                dmm::Map::from_file(Path::new(&file.filename)).map_err(|e| anyhow::anyhow!(e))?;
            let size = map.dim_xyz();
            let bb = BoundingBox {
                left: 0,
                bottom: 0,
                top: size.1 - 1,
                right: size.0 - 1,
            };
            maps.push(map);
            bbs.push(bb);
        }
        do_render(
            &base_context,
            &maps,
            &bbs,
            base_render_passes,
            removed_directory,
            "removed.png",
            &errors,
        )?;
        Ok(())
    })?;

    eprintln!("Errors: ");
    for error in errors.read().unwrap().iter() {
        eprintln!("{}", error);
    }

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
        .output()?;

    let non_abs_directory = format!("images/{}/{}", job.base.repo.id, job.check_run_id);
    let directory = Path::new(&non_abs_directory).absolutize()?;
    let directory = directory.as_ref().to_str().ok_or(anyhow::anyhow!(
        "Failed to create absolute path to image directory",
    ))?;

    git_checkout(
        &job.base
            .repo
            .default_branch
            .clone()
            .unwrap_or("master".to_owned()),
    )?;

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
        &base,
        &head,
        &added_files,
        &modified_files,
        &removed_files,
        Path::new(directory),
        job.pull_request,
    )?;

    let conf = CONFIG.read().await;
    let file_url = &conf.as_ref().unwrap().file_hosting_url;

    let title = "Map renderings";
    let summary = "Maps with diff:";
    let mut text = String::new();

    //TODO: split into added, modified, removed earlier

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
