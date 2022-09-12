use crate::github::{
    github_api::CheckRun,
    github_types::{self, Branch, CheckOutputs, FileDiff},
};
use anyhow::Result;
use octocrab::models::InstallationId;
use serde::{Deserialize, Serialize};
use yaque::Sender;

pub trait JobRunner: Fn(Job) -> Result<CheckOutputs> + Send + Clone + 'static {}
impl<T> JobRunner for T where T: Fn(Job) -> Result<CheckOutputs> + Send + Clone + 'static {}

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

pub type JobSender = Sender;
