use super::job_processor::do_job;
use diffbot_lib::job::types::Job;

pub async fn handle_jobs<S: AsRef<str>>(name: S, mut job_receiver: yaque::Receiver) {
    while let Ok(jobguard) = job_receiver.recv().await {
        let job = rmp_serde::from_slice(&jobguard);
        match job {
            Ok(job) => job_handler(name.as_ref(), job).await,
            Err(err) => eprintln!("{}", err),
        }
        if let Err(err) = jobguard.commit() {
            eprintln!("{}", err)
        };
    }
}

async fn job_handler(name: &str, job: Job) {
    let (repo, pull_request, check_run) =
        (job.repo.clone(), job.pull_request, job.check_run.clone());
    println!(
        "[{}#{}] [{}] Starting",
        repo.full_name(),
        pull_request,
        check_run.id()
    );

    let _ = check_run.mark_started().await;

    let output = actix_web::rt::task::spawn_blocking(move || do_job(job)).await;

    println!(
        "[{}#{}] [{}] Finished",
        repo.full_name(),
        pull_request,
        check_run.id()
    );

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

    let output = output.unwrap();
    diffbot_lib::job::runner::handle_output(output, check_run, name).await;
}
