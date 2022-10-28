use std::time::Duration;

use super::job_processor::do_job;
use diffbot_lib::job::types::Job;

use diffbot_lib::log::{error, info};

pub async fn handle_jobs<S: AsRef<str>>(name: S, mut job_receiver: yaque::Receiver) {
    loop {
        match job_receiver.recv().await {
            Ok(jobguard) => {
                info!("Job received from queue");
                let job = rmp_serde::from_slice(&jobguard);
                match job {
                    Ok(job) => job_handler(name.as_ref(), job).await,
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

    let output = actix_web::rt::time::timeout(
        Duration::from_secs(3600),
        actix_web::rt::task::spawn_blocking(move || do_job(job)),
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
