use std::path::PathBuf;
use std::time::Duration;

use super::job_processor::do_job;
use diffbot_lib::job::types::{Job, JobType};

use diffbot_lib::tracing;

use super::Azure;

pub async fn handle_jobs<S: AsRef<str>>(
    name: S,
    mut job_receiver: yaque::Receiver,
    blob_client: Azure,
) {
    loop {
        match job_receiver.recv().await {
            Ok(jobguard) => {
                tracing::info!("Job received from queue");
                let job: Result<JobType, serde_json::Error> = serde_json::from_slice(&jobguard);
                match job {
                    Ok(job) => match job {
                        JobType::GithubJob(job) => {
                            job_handler(name.as_ref(), *job, blob_client.clone()).await
                        }
                        JobType::CleanupJob(_) => garbage_collect_all_repos().await,
                    },
                    Err(err) => tracing::error!("Failed to parse job from queue: {err}"),
                }
                if let Err(err) = jobguard.commit() {
                    tracing::error!("Failed to commit change to queue: {err}")
                };
            }
            Err(err) => tracing::error!("{err}"),
        }
    }
}

async fn garbage_collect_all_repos() {
    use eyre::Result;
    use path_absolutize::Absolutize;
    use std::process::Command;
    tracing::info!("Garbage collection starting!");

    let output = actix_web::rt::time::timeout(
        Duration::from_secs(10800),
        //tfw no try blocks
        actix_web::rt::task::spawn_blocking(move || -> Result<()> {
            let path = PathBuf::from("./repos");
            if !path.exists() {
                tracing::info!("Repo path doesn't exist, skipping GC");
                return Ok(());
            }
            for entry in walkdir::WalkDir::new(path).min_depth(2).max_depth(2) {
                match entry {
                    Ok(entry) => {
                        let path = entry.into_path();
                        //tfw no try blocks
                        if let Err(err) = || -> Result<()> {
                            let path = path.absolutize()?;
                            let output =
                                Command::new("git").current_dir(&path).arg("gc").status()?;
                            if !output.success() {
                                match output.code() {
                                    Some(num) => tracing::error!(
                                        "GC failed on dir {} with code {num}",
                                        path.display(),
                                    ),
                                    None => tracing::error!(
                                        "GC failed on dir {}, process terminated!",
                                        path.display(),
                                    ),
                                }
                            }
                            Ok(())
                        }() {
                            tracing::error!("{err}");
                        }
                    }
                    Err(err) => tracing::error!("Walkdir failed: {err}"),
                }
            }
            Ok(())
        }),
    )
    .await;

    tracing::info!("Garbage collection finished!");

    let output = {
        if output.is_err() {
            tracing::error!("GC timed out!");
            return;
        }
        output.unwrap()
    };

    if let Err(e) = output {
        let fuckup = match e.try_into_panic() {
            Ok(panic) => {
                format!("{panic:#?}")
            }
            Err(e) => e.to_string(),
        };
        tracing::error!("Join Handle error: {fuckup}");
        return;
    }

    let output = output.unwrap();
    if let Err(e) = output {
        let fuckup = format!("{e:?}");
        tracing::error!("GC errored: {fuckup}");
    }
}

async fn job_handler(name: &str, job: Job, blob_client: Azure) {
    let (repo, pull_request, check_run) =
        (job.repo.clone(), job.pull_request, job.check_run.clone());
    tracing::info!(
        "[{}#{pull_request}] [{}] Starting",
        repo.full_name(),
        check_run.id()
    );

    let _ = check_run.mark_started().await;

    let output = actix_web::rt::time::timeout(
        Duration::from_secs(7200),
        actix_web::rt::task::spawn_blocking(move || do_job(job, blob_client)),
    )
    .await;

    tracing::info!(
        "[{}#{pull_request}] [{}] Finished",
        repo.full_name(),
        check_run.id()
    );

    let output = {
        if output.is_err() {
            tracing::error!("Job timed out!");
            let _ = check_run.mark_failed("Job timed out after 1 hours!").await;
            return;
        }
        output.unwrap()
    };

    if let Err(e) = output {
        let fuckup = match e.try_into_panic() {
            Ok(panic) => {
                format!("{panic:#?}")
            }
            Err(e) => e.to_string(),
        };
        tracing::error!("Join Handle error: {fuckup}");
        let _ = check_run.mark_failed(&fuckup).await;
        return;
    }

    let output = output.unwrap();
    if let Err(e) = output {
        let fuckup = format!("{e:?}");
        tracing::error!("Other rendering error: {fuckup}");
        let _ = check_run.mark_failed(&fuckup).await;
        return;
    }

    let output = output.unwrap();
    if let Err(e) = diffbot_lib::job::runner::handle_output(output, &check_run, name).await {
        let fuckup = format!("{e:?}");
        tracing::error!("Output upload error: {fuckup}");
        _ = check_run
            .mark_failed(&format!("Failed to upload job output: {fuckup}"))
            .await;
        return;
    };
}
