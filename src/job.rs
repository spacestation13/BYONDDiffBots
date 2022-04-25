use crate::github_types::*;
use flume::Sender;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Debug)]
pub struct Job {
    pub base: Branch,
    pub head: Branch,
    pub pull_request: u64,
    pub files: Vec<ModifiedFile>,
    pub check_run_id: u64,
    pub installation_id: u64,
}

pub struct JobSender(pub Sender<Job>);
