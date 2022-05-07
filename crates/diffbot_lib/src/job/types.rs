use std::collections::VecDeque;

use crate::github::{github_api::*, github_types::*};
use anyhow::{Context, Result};
use flume::Sender;
use serde::{Deserialize, Serialize};

pub trait JobRunner: Fn(&Job) -> Result<CheckOutputs> + Send + Clone + 'static {}
impl<T> JobRunner for T where T: Fn(&Job) -> Result<CheckOutputs> + Send + Clone + 'static {}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Job {
    pub base: Branch,
    pub head: Branch,
    pub pull_request: u64,
    pub files: Vec<ModifiedFile>,
    pub check_run: CheckRun,
}

pub struct JobSender(pub Sender<Job>);

//TODO: Integrate journaling and channel into some sort of queue?
pub struct JobJournal {
    file: String,
    jobs: VecDeque<Job>,
}

impl JobJournal {
    pub async fn from_file(file: &str) -> Result<Self> {
        // TODO: maybe we should report if the file doesn't exist?
        let jobs = tokio::fs::read_to_string(file)
            .await
            .unwrap_or_else(|_| "[]".to_owned());
        let jobs: VecDeque<Job> = serde_json::from_str(&jobs).unwrap_or_default();
        Ok(Self {
            file: file.to_owned(),
            jobs,
        })
    }

    pub fn get_job_count(&self) -> usize {
        self.jobs.len()
    }

    pub fn get_job(&self) -> Option<Job> {
        self.jobs.get(0).map(Clone::clone)
    }

    // Jobs are processed one at a time, so we can just remove the first job.
    pub async fn complete_job(&mut self) {
        self.jobs.pop_front();
        self.save().await.unwrap();
    }

    pub async fn add_job(&mut self, job: Job) {
        self.jobs.push_back(job);
        self.save().await.unwrap();
    }

    pub async fn save(&self) -> Result<()> {
        let jobs = serde_json::to_string(&self.jobs)?;
        tokio::fs::write(&self.file, jobs)
            .await
            .context("Saving job journal")?;
        Ok(())
    }
}
