use rocket::serde::Deserialize;
use serde::Serialize;

#[derive(Deserialize, Debug, Clone)]
pub struct User {
    pub login: String,
}

#[derive(Deserialize, Debug, Clone)]
pub struct Installation {
    pub id: u64,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Repository {
    pub url: String,
    pub name: String,
    pub id: u64,
    pub default_branch: Option<String>,
}

impl Repository {
    pub fn full_name(&self) -> (String, String) {
        let mut iter = self.url.split('/').skip(4).take(2).map(|a| a.to_string());
        (iter.next().unwrap(), iter.next().unwrap())
    }
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Branch {
    #[serde(rename = "ref")]
    pub name: String,
    pub repo: Repository,
    pub sha: String,
}

#[derive(Deserialize, Debug, Clone)]
pub struct PullRequest {
    pub number: u64,
    pub head: Branch,
    pub base: Branch,
    pub title: Option<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "snake_case")]
pub enum ModifiedFileStatus {
    Added,
    Removed,
    Modified,
    Renamed,
    Copied,
    Changed,
    Unchanged,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ModifiedFile {
    pub filename: String,
    pub status: ModifiedFileStatus,
    pub sha: String,
}

#[derive(Deserialize, Debug)]
pub struct CheckSuite {
    pub id: u64,
    pub pull_requests: Vec<PullRequest>,
    pub head_sha: String,
}

#[derive(Deserialize, Debug)]
pub struct App {
    pub id: u64,
    pub name: String,
}

#[derive(Deserialize, Debug)]
pub struct RawCheckRun {
    pub id: u64,
    pub pull_requests: Vec<PullRequest>,
    pub head_sha: String,
    pub app: App,
}

#[derive(Deserialize, Debug)]
pub struct CheckSuitePayload {
    pub action: String,
    pub repository: Repository,
    pub check_suite: CheckSuite,
    pub installation: Installation,
}

#[derive(Deserialize, Debug)]
pub struct CheckRunPayload {
    pub action: String,
    pub repository: Repository,
    pub check_run: RawCheckRun,
    pub installation: Installation,
}

#[derive(Deserialize, Debug)]
pub struct PullRequestEventPayload {
    pub action: String,
    pub number: u64,
    pub repository: Repository,
    pub pull_request: PullRequest,
    pub installation: Installation,
}
