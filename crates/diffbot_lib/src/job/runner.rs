use std::sync::Arc;

use flume::Receiver;
use tokio::sync::Mutex;

use crate::job::types::{Job, JobRunner};

use super::types::JobJournal;

async fn handle_job<S: AsRef<str>, F>(name: S, job: Job, runner: F)
where
    F: JobRunner,
{
    let (repo, pull_request, check_run) =
        (job.repo.clone(), job.pull_request, job.check_run.clone());
    println!(
        "[{}] [{}#{}] [{}] Starting",
        chrono::Utc::now().to_rfc3339(),
        repo.full_name(),
        pull_request,
        check_run.id()
    );

    let _ = check_run.mark_started().await; // TODO: Put the failed marks in a queue to retry later
                                            //let now = Instant::now();

    let output = tokio::task::spawn_blocking(move || runner(job)).await;

    println!(
        "[{}] [{}#{}] [{}] Finished",
        chrono::Utc::now().to_rfc3339(),
        repo.full_name(),
        pull_request,
        check_run.id()
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
        let _ = check_run.mark_failed(&fuckup).await;
        return;
    }

    let output = output.unwrap();
    if let Err(e) = output {
        let fuckup = format!("{:?}", e);
        eprintln!("Other rendering error: {}", fuckup);
        let _ = check_run.mark_failed(&fuckup).await;
        return;
    }
    let mut output = output.unwrap();

    match output.len() {
        0usize => {
            let _ = check_run.mark_failed("Rendering returned nothing!").await;
        }
        1usize => {
            let res = check_run.mark_succeeded(output.pop().unwrap()).await;
            if res.is_err() {
                let _ = check_run
                    .mark_failed(&format!("Failed to upload job output: {:?}", res))
                    .await;
            }
        }
        _ => {
            let len = output.len();
            for (idx, item) in output.into_iter().enumerate() {
                match idx {
                    0usize => {
                        let _ = check_run
                            .rename(&format!("{} (1/{})", name.as_ref(), len))
                            .await;
                        let res = check_run.mark_succeeded(item).await;
                        if res.is_err() {
                            let _ = check_run
                                .mark_failed(&format!("Failed to upload job output: {:?}", res))
                                .await;
                            return;
                        }
                    }
                    _ => {
                        if let Ok(check) = check_run
                            .duplicate(&format!("{} ({}/{})", name.as_ref(), idx + 1, len))
                            .await
                        {
                            let res = check.mark_succeeded(item).await;
                            if res.is_err() {
                                let _ = check_run
                                    .mark_failed(&format!("Failed to upload job output: {:?}", res))
                                    .await;
                                return;
                            }
                        }
                    }
                }
            }
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
