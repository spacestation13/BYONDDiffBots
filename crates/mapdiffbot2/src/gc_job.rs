use delay_timer::prelude::*;
use diffbot_lib::{
    job::types::{JobSender, JobType},
    tracing,
};

pub async fn gc_scheduler(cron_str: String, job: JobSender<JobType>) {
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
                    async move {
                        if let Err(err) = sender_clone.send_async(JobType::CleanupJob).await {
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
