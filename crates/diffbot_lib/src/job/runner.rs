use std::sync::Arc;

use flume::Receiver;
use tokio::sync::Mutex;

use crate::{
    github::github_types::CheckOutputs,
    job::types::{Job, JobRunner},
};

use super::types::JobJournal;

async fn handle_job<S: AsRef<str>, F>(name: S, job: Job, runner: F)
where
    F: JobRunner,
{
    println!(
        "[{}] [{}#{}] [{}] Starting",
        chrono::Utc::now().to_rfc3339(),
        job.repo.full_name(),
        job.pull_request,
        job.check_run.id()
    );
    let _ = job.check_run.mark_started().await; // TODO: Put the failed marks in a queue to retry later
                                                //let now = Instant::now();

    let job_clone = job.clone();
    let output = tokio::task::spawn_blocking(move || runner(&job_clone)).await;

    println!(
        "[{}] [{}#{}] [{}] Finished",
        chrono::Utc::now().to_rfc3339(),
        job.repo.full_name(),
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
            let res = job.check_run.mark_succeeded(output).await;
            if res.is_err() {
                let _ = job
                    .check_run
                    .mark_failed(&format!("Failed to upload job output: {:?}", res))
                    .await;
            }
        }
        CheckOutputs::Many(mut outputs) => {
            let count = outputs.len();

            let _ = job
                .check_run
                .rename(&format!("{} (1/{})", name.as_ref(), count))
                .await;
            let res = job.check_run.mark_succeeded(outputs.remove(0)).await;
            if res.is_err() {
                let _ = job
                    .check_run
                    .mark_failed(&format!("Failed to upload job output: {:?}", res))
                    .await;
                return;
            }

            for (i, overflow) in outputs.into_iter().enumerate() {
                if let Ok(check) = job
                    .check_run
                    .duplicate(&format!("{} ({}/{})", name.as_ref(), i + 2, count))
                    .await
                {
                    let res = check.mark_succeeded(overflow).await;
                    if res.is_err() {
                        let _ = job
                            .check_run
                            .mark_failed(&format!("Failed to upload job output: {:?}", res))
                            .await;
                        return;
                    }
                }
            }
        }
        CheckOutputs::None => {
            let _ = job
                .check_run
                .mark_failed("Rendering returned nothing!")
                .await;
        }
    }
}

async fn recover_from_journal<S: AsRef<str>, F>(
    name: S,
    journal: &Arc<Mutex<JobJournal>>,
    runner: F,
) where
    F: JobRunner,
{
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
            handle_job(&name, job, runner.clone()).await;
            journal.lock().await.complete_job().await;
        } else {
            break;
        }
    }
}

pub async fn handle_jobs<S: AsRef<str>, F>(
    name: S,
    job_receiver: Receiver<Job>,
    journal: Arc<Mutex<JobJournal>>,
    runner: F,
) where
    F: JobRunner,
{
    recover_from_journal(&name, &journal, runner.clone()).await;
    loop {
        let job = job_receiver.recv_async().await;
        if let Ok(job) = job {
            handle_job(&name, job, runner.clone()).await;
            journal.lock().await.complete_job().await;
        } else {
            break;
        }
    }
}
