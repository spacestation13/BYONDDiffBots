use std::time::Duration;

use super::job_processor::do_job;
use diffbot_lib::job::types::Job;

use diffbot_lib::tracing;

pub async fn handle_jobs<S: AsRef<str>>(
    name: S,
    job_receiver: flume::Receiver<Job>,
    client: reqwest::Client,
) {
    loop {
        match job_receiver.recv_async().await {
            Ok(job) => {
                tracing::info!("Job received from queue");
                job_handler(name.as_ref(), job, client.clone()).await;
            }
            Err(err) => tracing::error!("{err}"),
        }
    }
}

async fn job_handler(name: &str, job: Job, client: reqwest::Client) {
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
        actix_web::rt::task::spawn_blocking(move || do_job(job, client)),
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
    };
}
