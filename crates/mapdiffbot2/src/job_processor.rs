use anyhow::{Context, Result};
use flume::Receiver;
use path_absolutize::*;
use rayon::prelude::*;
use rocket::tokio::runtime::Handle;
use rocket::tokio::sync::Mutex;
use rocket::tokio::task;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;
use std::sync::Arc;
//use std::time::Instant;

extern crate dreammaker as dm;

use crate::job::Job;
use crate::rendering::*;
use crate::{job, CONFIG};
use diffbot_lib::git::git_operations::*;
use diffbot_lib::github::github_types::*;

struct RenderedMaps {
    added_maps: Vec<MapWithRegions>,
    removed_maps: Vec<MapWithRegions>,
    modified_maps: MapsWithRegions,
}

fn render(
    base: &Branch,
    head: &Branch,
    added_files: &[&ModifiedFile],
    modified_files: &[&ModifiedFile],
    removed_files: &[&ModifiedFile],
    output_dir: &Path,
    pull_request_number: u64,
    // feel like this is a bit of a hack but it works for now
) -> Result<RenderedMaps> {
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
    let modified_maps = get_map_diff_bounding_boxes(base_maps, head_maps);

    //let now = Instant::now();
    // You might think to yourself, wtf is going on here?
    // And you'd be right.
    let removed_maps = with_repo_dir(&base.repo, || {
        let results = rayon::join(
            || {
                render_map_regions(
                    &base_context,
                    &modified_maps.befores,
                    &head_render_passes,
                    modified_directory,
                    "before.png",
                    &modified_before_errors,
                )
                .context("Rendering modified before maps")
            },
            || {
                let maps = load_maps_with_whole_map_regions(removed_files)
                    .context("Loading removed maps")?;

                render_map_regions(
                    &base_context,
                    &maps,
                    &base_render_passes,
                    removed_directory,
                    "removed.png",
                    &removed_errors,
                )
                .context("Rendering removed maps")?;

                Ok(maps)
            },
        );
        results.0?;
        results.1
        //eprintln!("Base maps took {}ms", now.elapsed().as_millis());
    })?;

    //let now = Instant::now();
    let added_maps = with_checkout(&base.repo, &pull_branch, || {
        let results = rayon::join(
            || {
                render_map_regions(
                    &head_context,
                    &modified_maps.afters,
                    &head_render_passes,
                    modified_directory,
                    "after.png",
                    &modified_after_errors,
                )
            },
            || {
                let maps =
                    load_maps_with_whole_map_regions(added_files).context("Loading added maps")?;

                render_map_regions(
                    &head_context,
                    &maps,
                    &head_render_passes,
                    added_directory,
                    "added.png",
                    &added_errors,
                )
                .context("Rendering added maps")?;

                Ok(maps)
            },
        );
        results.0?; // Is there a better way?
        results.1
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

    //let now = Instant::now();
    (0..modified_files.len()).into_par_iter().for_each(|i| {
        let _ = render_diffs_for_directory(modified_directory.join(i.to_string()));
    });
    /*eprintln!(
        "Generating {} diff(s) took {}ms",
        modified_files.len(),
        now.elapsed().as_millis()
    );*/

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

    Ok(RenderedMaps {
        added_maps,
        modified_maps,
        removed_maps,
    })
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

enum CheckOutputs {
    One(Output),
    Many(Output, Vec<Output>),
}

struct CheckOutputBuilder {
    title: String,
    summary: String,
    current_text: String,
    outputs: Vec<Output>,
}

impl CheckOutputBuilder {
    pub fn new<S: Into<String>>(title: S, summary: S) -> Self {
        let title = title.into();
        let summary = summary.into();
        Self {
            title,
            summary,
            current_text: String::new(),
            outputs: Vec::new(),
        }
    }

    pub fn add_text(&mut self, text: &str) {
        self.current_text.push_str(text);
        // Leaving a 5k character safety margin is prob overkill but oh well
        if self.current_text.len() > 60_000 {
            let output = Output {
                title: self.title.clone(),
                summary: self.summary.clone(),
                text: std::mem::take(&mut self.current_text),
            };
            self.outputs.push(output);
        }
    }

    pub fn build(self) -> CheckOutputs {
        let Self {
            title,
            summary,
            current_text,
            mut outputs,
        } = self;

        if !current_text.is_empty() {
            let output = Output {
                title,
                summary,
                text: current_text,
            };
            outputs.push(output);
        }
        let first = outputs.remove(0);
        if outputs.is_empty() {
            CheckOutputs::One(first)
        } else {
            CheckOutputs::Many(first, outputs)
        }
    }
}

fn generate_finished_output<P: AsRef<Path>>(
    added_files: &[&ModifiedFile],
    modified_files: &[&ModifiedFile],
    removed_files: &[&ModifiedFile],
    file_directory: &P,
    maps: RenderedMaps,
) -> Result<CheckOutputs> {
    let conf = CONFIG.get().unwrap();
    let file_url = &conf.file_hosting_url;
    let non_abs_directory = file_directory.as_ref().to_string_lossy();

    let mut builder = CheckOutputBuilder::new(
    "Map renderings",
    "*This is still a beta. Please file any issues [here](https://github.com/MCHSL/mapdiffbot2/issues).*\n\nMaps with diff:",
	);

    let link_base = format!("{}/{}", file_url, non_abs_directory);

    // Those are CPU bound but parallelizing would require builder to be thread safe and it's probably not worth the overhead
    added_files
        .iter()
        .zip(maps.added_maps.iter())
        .enumerate()
        .for_each(|(file_index, (file, map))| {
            map.iter_levels().for_each(|(level, _)| {
                let link = format!("{}/a/{}/{}-added.png", link_base, file_index, level);
                let name = format!("{}:{}", file.filename, level + 1);

                builder.add_text(&format!(
                    include_str!("diff_template_add.txt"),
                    filename = name,
                    image_link = link
                ));
            });
        });

    modified_files
        .iter()
        .zip(maps.modified_maps.befores.iter())
        .enumerate()
        .for_each(|(file_index, (file, map))| {
            map.iter_levels().for_each(|(level, region)| {
                let link = format!("{}/m/{}/{}", link_base, file_index, level);
                let name = format!("{}:{}", file.filename, level + 1);

                #[allow(clippy::format_in_format_args)]
                builder.add_text(&format!(
                    include_str!("diff_template_mod.txt"),
                    bounds = region.to_string(),
                    filename = name,
                    image_before_link = format!("{}-before.png", link),
                    image_after_link = format!("{}-after.png", link),
                    image_diff_link = format!("{}-diff.png", link)
                ));
            });
        });

    removed_files
        .iter()
        .zip(maps.removed_maps.iter())
        .enumerate()
        .for_each(|(file_index, (file, map))| {
            map.iter_levels().for_each(|(level, _)| {
                let link = format!("{}/r/{}/{}-removed.png", link_base, file_index, level);
                let name = format!("{}:{}", file.filename, level + 1);

                builder.add_text(&format!(
                    include_str!("diff_template_remove.txt"),
                    filename = name,
                    image_link = link
                ));
            });
        });

    Ok(builder.build())
}

fn do_job(job: &Job) -> Result<CheckOutputs> {
    let base = &job.base;
    let head = &job.head;
    let repo = format!("https://github.com/{}", base.repo.full_name());
    let target_dir: PathBuf = ["./repos/", &base.repo.name].iter().collect();

    if !target_dir.exists() {
        if let Ok(handle) = Handle::try_current() {
            handle.block_on(async {
				let output = Output {
					title: "Cloning repo...".to_owned(),
					summary: "The repository is being cloned, this will take a few minutes. Future runs will not require cloning.".to_owned(),
					text: "".to_owned(),
				};
				let _ = job.check_run.set_output(output).await; // we don't really care if updating the job fails, just continue
			});
        }

        clone_repo(&repo, &target_dir).context("Cloning repo")?;
    }

    let non_abs_directory = format!("images/{}/{}", job.base.repo.id, job.check_run.id());
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

    let maps = render(
        base,
        head,
        &added_files,
        &modified_files,
        &removed_files,
        Path::new(directory),
        job.pull_request,
    )
    .context("Doing the renderance")?;

    let outputs = generate_finished_output(
        &added_files,
        &modified_files,
        &removed_files,
        &non_abs_directory,
        maps,
    )?;

    Ok(outputs)
}

async fn handle_job(job: job::Job) {
    println!(
        "[{}] [{}#{}] [{}] Starting",
        chrono::Utc::now().to_rfc3339(),
        job.base.repo.full_name(),
        job.pull_request,
        job.check_run.id()
    );
    let _ = job.check_run.mark_started().await; // TODO: Put the failed marks in a queue to retry later
                                                //let now = Instant::now();

    let job_clone = job.clone();
    let output = task::spawn_blocking(move || do_job(&job_clone)).await;

    println!(
        "[{}] [{}#{}] [{}] Finished",
        chrono::Utc::now().to_rfc3339(),
        job.base.repo.full_name(),
        job.pull_request,
        job.check_run.id()
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
        let _ = job.check_run.mark_failed(&fuckup).await;
        return;
    }

    let output = output.unwrap();
    if let Err(e) = output {
        let fuckup = format!("{:?}", e);
        eprintln!("Other rendering error: {}", fuckup);
        let _ = job.check_run.mark_failed(&fuckup).await;
        return;
    }

    match output.unwrap() {
        CheckOutputs::One(output) => {
            let _ = job.check_run.mark_succeeded(output).await;
        }
        CheckOutputs::Many(first, rest) => {
            let count = rest.len() + 1;

            let _ = job
                .check_run
                .rename(&format!("MapDiffBot2 (1/{})", count))
                .await;
            let _ = job.check_run.mark_succeeded(first).await;

            for (i, overflow) in rest.into_iter().enumerate() {
                if let Ok(check) = job
                    .check_run
                    .duplicate(&format!("MapDiffBot2 ({}/{})", i + 2, count))
                    .await
                {
                    let _ = check.mark_succeeded(overflow).await;
                }
            }
        }
    }
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
