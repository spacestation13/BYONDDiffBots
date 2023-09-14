use std::sync::Arc;

use delay_timer::prelude::*;
use diffbot_lib::{
    async_mutex::Mutex,
    job::types::{JobSender, JobType},
    tracing,
};

pub async fn gc_scheduler(cron_str: String, job: Arc<Mutex<JobSender>>) {
    let scheduler = DelayTimerBuilder::default()
        .tokio_runtime_by_default()
        .build();
    scheduler
        .add_task(
            TaskBuilder::default()
                .set_frequency_repeated_by_cron_str(cron_str.as_str())
                .set_maximum_parallel_runnable_num(1)
                .set_task_id(1)
                .spawn_async_routine(move || {
                    let sender_clone = job.clone();
                    let job =
                        serde_json::to_vec(&JobType::CleanupJob("GC_REQUEST_DUMMY".to_owned()))
                            .expect("Cannot serialize cleanupjob, what the fuck");
                    async move {
                        if let Err(err) = sender_clone.lock().await.send(job).await {
                            tracing::error!("Cannot send cleanup job: {err}")
                        }
                    }
                })
                .expect("Can't create Cron task"),
        )
        .expect("cannot add cron job, FUCK");
    actix_web::rt::signal::ctrl_c()
        .await
        .expect("Cannot wait for sigterm");
    scheduler.remove_task(1).expect("Can't remove task");
    scheduler
        .stop_delay_timer()
        .expect("Can't stop delaytimer, FUCK");
}
