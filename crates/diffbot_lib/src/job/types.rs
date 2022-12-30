use crate::github::{
    github_api::CheckRun,
    github_types::{self, Branch, CheckOutputs, FileDiff},
};
use eyre::Result;
use octocrab::models::InstallationId;
use serde::{Deserialize, Serialize};
use yaque::Sender;

pub type JobRunner = fn(Job) -> Result<CheckOutputs>;

pub type JobSender = Sender;

#[derive(Serialize, Deserialize, Debug, Clone)]
pub enum JobType {
    GithubJob(Job),
    CleanupJob(String),
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Job {
    pub repo: github_types::Repository,
    pub base: Branch,
    pub head: Branch,
    pub pull_request: u64,
    pub files: Vec<FileDiff>,
    pub check_run: CheckRun,
    pub installation: InstallationId,
}
