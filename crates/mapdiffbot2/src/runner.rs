use std::time::Duration;

use super::job_processor::do_job;
use diffbot_lib::job::types::{Job, JobType};

use diffbot_lib::log::{error, info};

pub async fn handle_jobs<S: AsRef<str>>(name: S, mut job_receiver: yaque::Receiver) {
    loop {
        match job_receiver.recv().await {
            Ok(jobguard) => {
                info!("Job received from queue");
                let job: Result<JobType, serde_json::Error> = serde_json::from_slice(&jobguard);
                match job {
                    Ok(job) => match job {
                        JobType::GithubJob(job) => job_handler(name.as_ref(), job).await,
                        JobType::CleanupJob => garbage_collect_all_repos().await,
                    },
                    Err(err) => error!("Failed to parse job from queue: {}", err),
                }
                if let Err(err) = jobguard.commit() {
                    error!("Failed to commit change to queue: {}", err)
                };
            }
            Err(err) => {
                error!("Cannot receive jobs from queue: {}", err)
            }
        }
    }
}

async fn garbage_collect_all_repos() {
    use eyre::Result;
    use path_absolutize::Absolutize;
    use std::process::Command;
    info!("Garbage collection starting!");

    let output = rocket::tokio::time::timeout(
        Duration::from_secs(3600),
        //tfw no try blocks
        rocket::tokio::task::spawn_blocking(move || -> Result<()> {
            std::env::set_current_dir(
                std::env::current_exe()?
                    .parent()
                    .expect("Couldn't find the current exe's parent dir"),
            )?;
            for entry in walkdir::WalkDir::new("./repos").min_depth(2).max_depth(2) {
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
                                    Some(num) => error!(
                                        "GC failed on dir {} with code {}",
                                        path.display(),
                                        num
                                    ),
                                    None => error!(
                                        "GC failed on dir {}, process terminated!",
                                        path.display(),
                                    ),
                                }
                            }
                            Ok(())
                        }() {
                            error!("{}", err);
                        }
                    }
                    Err(err) => error!("Walkdir failed: {}", err),
                }
            }
            Ok(())
        }),
    )
    .await;

    info!("Garbage collection finished!");

    let output = {
        if let Err(_) = output {
            error!("GC timed out!");
            return;
        }
        output.unwrap()
    };

    if let Err(e) = output {
        let fuckup = match e.try_into_panic() {
            Ok(panic) => match panic.downcast_ref::<&str>() {
                Some(s) => s.to_string(),
                None => "*crickets*".to_owned(),
            },
            Err(e) => e.to_string(),
        };
        error!("Join Handle error: {}", fuckup);
        return;
    }

    let output = output.unwrap();
    if let Err(e) = output {
        let fuckup = format!("{:?}", e);
        error!("GC errored: {}", fuckup);
        return;
    }
}

async fn job_handler(name: &str, job: Job) {
    let (repo, pull_request, check_run) =
        (job.repo.clone(), job.pull_request, job.check_run.clone());
    info!(
        "[{}#{}] [{}] Starting",
        repo.full_name(),
        pull_request,
        check_run.id()
    );

    let _ = check_run.mark_started().await;

    let output = rocket::tokio::time::timeout(
        Duration::from_secs(3600),
        rocket::tokio::task::spawn_blocking(move || do_job(job)),
    )
    .await;

    info!(
        "[{}#{}] [{}] Finished",
        repo.full_name(),
        pull_request,
        check_run.id()
    );

    let output = {
        if let Err(_) = output {
            error!("Job timed out!");
            let _ = check_run.mark_failed("Job timed out after 1 hours!").await;
            return;
        }
        output.unwrap()
    };

    if let Err(e) = output {
        let fuckup = match e.try_into_panic() {
            Ok(panic) => match panic.downcast_ref::<&str>() {
                Some(s) => s.to_string(),
                None => "*crickets*".to_owned(),
            },
            Err(e) => e.to_string(),
        };
        error!("Join Handle error: {}", fuckup);
        let _ = check_run.mark_failed(&fuckup).await;
        return;
    }

    let output = output.unwrap();
    if let Err(e) = output {
        let fuckup = format!("{:?}", e);
        error!("Other rendering error: {}", fuckup);
        let _ = check_run.mark_failed(&fuckup).await;
        return;
    }

    let output = output.unwrap();
    diffbot_lib::job::runner::handle_output(output, check_run, name).await;
}
