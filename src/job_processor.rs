use anyhow::{Context, Result};
use flume::Receiver;
use path_absolutize::*;
use rocket::tokio::runtime::Handle;
use rocket::tokio::sync::Mutex;
use rocket::tokio::task;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;
use std::sync::Arc;
//use std::time::Instant;

extern crate dreammaker as dm;

use crate::git_operations::*;
use crate::github_api::*;
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
    with_repo_dir(&base.repo, || {
        Command::new("git")
            .args(["checkout", &base.name])
            .output()
            .context("Running base checkout command")?;

        Command::new("git")
            .args(["pull", "origin", &base.name])
            .output()
            .context("Running base pull command")?;

        let output = Command::new("git")
            .args(["branch"])
            .output()
            .context("Running branch command")?;

        String::from_utf8(output.stdout)?
            .lines()
            .map(|l| l.trim())
            .filter(|l| l.starts_with("mdb-"))
            .for_each(|l| {
                let _ = Command::new("git").args(["branch", "-D", l]).status();
            });

        Ok(())
    })
    .context("Updating to latest master on base")?;

    //let now = Instant::now();
    let path = format!("./repos/{}", &base.repo.name);
    let path = Path::new(&path)
        .absolutize()
        .context("Making repo path absolute")?;
    let base_context = RenderingContext::new(&path).context("Parsing base")?;
    //eprintln!("Parsing base took {}ms", now.elapsed().as_millis());

    let pull_branch = format!("mdb-{}-{}", base.sha, head.sha);
    let fetch_branch = format!("pull/{}/head:{}", pull_request_number, pull_branch);

    //let now = Instant::now();
    with_repo_dir(&base.repo, || {
        Command::new("git")
            .args(["fetch", "origin", &fetch_branch])
            .output()
            .context("Running fetch command")?;
        Ok(())
    })
    .context("Fetching the pull")?;

    let head_context = with_checkout(&base.repo, &pull_branch, || RenderingContext::new(&path))
        .context("Parsing head")?;

    // eprintln!(
    //     "Fetching and parsing head took {}ms",
    //     now.elapsed().as_millis()
    // );

    let base_render_passes = dmm_tools::render_passes::configure(
        base_context.map_config(),
        "",
        "hide-space,hide-invisible,random",
    );

    let head_render_passes = dmm_tools::render_passes::configure(
        head_context.map_config(),
        "",
        "hide-space,hide-invisible,random",
    );

    // ADDED MAPS
    let added_directory = format!("{}/a", output_dir.display());
    let added_directory = Path::new(&added_directory);
    let added_errors = Default::default();

    // MODIFIED MAPS
    let modified_directory = format!("{}/m", output_dir.display());
    let modified_directory = Path::new(&modified_directory);
    let modified_before_errors = Default::default();
    let modified_after_errors = Default::default();

    let removed_directory = format!("{}/r", output_dir.display());
    let removed_directory = Path::new(&removed_directory);
    let removed_errors = Default::default();

    let base_maps =
        with_repo_dir(&base.repo, || load_maps(modified_files)).context("Loading base maps")?;
    let head_maps = with_checkout(&base.repo, &pull_branch, || load_maps(modified_files))
        .context("Loading head maps")?;
    let diff_bounds = get_map_diff_bounding_boxes(&base_maps, &head_maps);

    //let now = Instant::now();
    with_repo_dir(&base.repo, || {
        let results = rayon::join(
            || {
                render_map_regions(
                    &base_context,
                    &base_maps,
                    &diff_bounds,
                    &head_render_passes,
                    modified_directory,
                    "before.png",
                    &modified_before_errors,
                )
                .context("Rendering modified before maps")
            },
            || {
                let maps = load_maps(removed_files).context("Loading removed maps")?;
                let bounds = maps
                    .iter()
                    .map(BoundingBox::for_full_map)
                    .collect::<Vec<BoundingBox>>();

                render_map_regions(
                    &base_context,
                    &maps,
                    &bounds,
                    &base_render_passes,
                    removed_directory,
                    "removed.png",
                    &removed_errors,
                )
                .context("Rendering removed maps")
            },
        );
        results.0?;
        results.1?;
        //eprintln!("Base maps took {}ms", now.elapsed().as_millis());
        Ok(())
    })?;

    //let now = Instant::now();
    with_checkout(&base.repo, &pull_branch, || {
        let results = rayon::join(
            || {
                render_map_regions(
                    &head_context,
                    &head_maps,
                    &diff_bounds,
                    &head_render_passes,
                    modified_directory,
                    "after.png",
                    &modified_after_errors,
                )
            },
            || {
                let maps = load_maps(added_files).context("Loading added maps")?;
                let bounds = maps
                    .iter()
                    .map(BoundingBox::for_full_map)
                    .collect::<Vec<BoundingBox>>();

                render_map_regions(
                    &head_context,
                    &maps,
                    &bounds,
                    &head_render_passes,
                    added_directory,
                    "added.png",
                    &added_errors,
                )
                .context("Rendering added maps")
            },
        );
        results.0?; // Is there a better way?
        results.1?;
        Ok(())
    })
    .context("Rendering modified after and added maps")?;
    //eprintln!("Head maps took {}ms", now.elapsed().as_millis());

    with_repo_dir(&base.repo, || {
        Command::new("git")
            .args(["branch", "-D", &pull_branch])
            .output()?;

        Ok(())
    })
    .context("Deleting pull branch")?;

    /*
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
    */

    Ok(())
}

fn clone_repo(url: &str, dir: &Path) -> Result<()> {
    Command::new("git")
        .args([
            "clone",
            url,
            dir.to_str()
                .ok_or_else(|| anyhow::anyhow!("Target directory is somehow unstringable"))?,
        ])
        .output()
        .context("Cloning repo")?;

    Ok(())
}

fn do_job(job: &Job) -> Result<Output> {
    let base = &job.base;
    let head = &job.head;
    let repo = format!("https://github.com/{}", base.repo.full_name());
    let target_dir: PathBuf = ["./repos/", &base.repo.name].iter().collect();

    if !target_dir.exists() {
        if let Ok(handle) = Handle::try_current() {
            let _ = handle.block_on(async {
				update_job(job, Output {
					title: "Cloning repo...".to_owned(),
					summary: "The repository is being cloned, this will take a few minutes. Future runs will not require cloning.".to_owned(),
					text: "".to_owned(),
				}).await // we don't really care if updating the job fails, just continue
			});
        }

        clone_repo(&repo, &target_dir).context("Cloning repo")?;
    }

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

    let conf = CONFIG.get().unwrap();
    let file_url = &conf.file_hosting_url;

    let title = "Map renderings";
    let summary = "*This is still a beta. Please file any issues [here](https://github.com/MCHSL/mapdiffbot2/issues).*\n\nMaps with diff:";
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

    Ok(output)
}

async fn handle_job(job: job::Job) {
    println!(
        "[{}] [{}#{}] [{}] Starting",
        chrono::Utc::now().to_rfc3339(),
        job.base.repo.full_name(),
        job.pull_request,
        job.check_run_id
    );
    let _ = mark_job_started(&job).await; // TODO: Put the failed marks in a queue to retry later
                                          //let now = Instant::now();

    let job_clone = job.clone();
    let output = task::spawn_blocking(move || do_job(&job_clone)).await;

    println!(
        "[{}] [{}#{}] [{}] Finished",
        chrono::Utc::now().to_rfc3339(),
        job.base.repo.full_name(),
        job.pull_request,
        job.check_run_id
    );

    //eprintln!("Handling job took {}ms", now.elapsed().as_millis());

    if let Err(e) = output {
        let fuckup = match e.try_into_panic() {
            Ok(panic) => match panic.downcast_ref::<&str>() {
                Some(s) => s.to_string(),
                None => "*crickets*".to_owned(),
            },
            Err(e) => e.to_string(),
        };
        eprintln!("Join Handle error: {}", fuckup);
        mark_job_failed(&job, &fuckup).await.unwrap();
        return;
    }

    let output = output.unwrap();
    if let Err(e) = output {
        let fuckup = format!("{:?}", e);
        eprintln!("Other rendering error: {}", fuckup);
        mark_job_failed(&job, &fuckup).await.unwrap();
        return;
    }

    let _ = mark_job_success(&job, output.unwrap()).await;
}

async fn recover_from_journal(journal: &Arc<Mutex<job::JobJournal>>) {
    let num_jobs = journal.lock().await.get_job_count();
    if num_jobs > 0 {
        eprintln!("Recovering {} jobs from journal", num_jobs);
    } else {
        eprintln!("No jobs to recover from journal");
        return;
    }

    loop {
        // Done this way to avoid a deadlock
        let job = journal.lock().await.get_job();
        if let Some(job) = job {
            handle_job(job).await;
            journal.lock().await.complete_job().await;
        } else {
            break;
        }
    }
}

pub async fn handle_jobs(job_receiver: Receiver<job::Job>, journal: Arc<Mutex<job::JobJournal>>) {
    recover_from_journal(&journal).await;
    loop {
        let job = job_receiver.recv_async().await;
        if let Ok(job) = job {
            handle_job(job).await;
            journal.lock().await.complete_job().await;
        } else {
            break;
        }
    }
}
