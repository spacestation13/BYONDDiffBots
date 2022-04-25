use crate::github_types::*;
use anyhow::{Context, Result};
use flume::Sender;
use rocket::serde::json::serde_json;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Job {
    pub base: Branch,
    pub head: Branch,
    pub pull_request: u64,
    pub files: Vec<ModifiedFile>,
    pub check_run_id: u64,
    pub installation_id: u64,
}

pub struct JobSender(pub Sender<Job>);

//TODO: Integrate journaling and channel into some sort of queue?
pub struct JobJournal {
    file: String,
    jobs: Vec<Job>,
}

impl JobJournal {
    pub async fn from_file(file: &str) -> Result<Self> {
        // TODO: maybe we should report if the file doesn't exist?
        let jobs = rocket::tokio::fs::read_to_string(file)
            .await
            .unwrap_or_else(|_| "[]".to_owned());
        let jobs: Vec<Job> = serde_json::from_str(&jobs).unwrap_or_else(|_| vec![]);
        Ok(Self {
            file: file.to_owned(),
            jobs,
        })
    }

    pub fn has_jobs(&self) -> bool {
        !self.jobs.is_empty()
    }

    pub async fn get_job(&self) -> Option<&Job> {
        self.jobs.get(0)
    }

    // Jobs are processed one at a time, so we can just remove the first job.
    pub async fn complete_job(&mut self) {
        self.jobs.remove(0);
        self.save().await.unwrap();
    }

    pub async fn add_job(&mut self, job: Job) {
        self.jobs.push(job);
        self.save().await.unwrap();
    }

    pub async fn save(&self) -> Result<()> {
        let jobs = serde_json::to_string(&self.jobs)?;
        rocket::tokio::fs::write(&self.file, jobs)
            .await
            .context("Saving job journal")?;
        Ok(())
    }
}
