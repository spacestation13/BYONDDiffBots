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

#[derive(Deserialize, Debug, Clone)]
pub struct Repository {
    pub url: String,
    pub name: String,
    pub id: u64,
    pub default_branch: Option<String>,
}

impl Repository {
    pub fn full_name(&self) -> String {
        self.url.split("/").skip(4).collect::<Vec<&str>>().join("/")
    }

    pub fn owner(&self) -> String {
        self.url
            .split("/")
            .skip(4)
            .take(1)
            .collect::<Vec<&str>>()
            .join("")
    }
}

#[derive(Deserialize, Debug, Clone)]
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
}
#[derive(Deserialize, Debug)]
pub struct ModifiedFile {
    pub filename: String,
    pub status: String,
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
pub struct CheckRun {
    pub id: u64,
    pub pull_requests: Vec<PullRequest>,
    pub head_sha: String,
    pub app: App,
    pub check_suite: CheckSuite,
}

#[derive(Deserialize, Debug)]
pub struct JobPayload {
    pub action: String,
    pub repository: Repository,
    pub check_suite: Option<CheckSuite>,
    pub check_run: Option<CheckRun>,
    pub installation: Installation,
}

#[derive(Serialize)]
pub struct Output {
    pub title: String,
    pub summary: String,
    pub text: String,
}

#[derive(Serialize)]
pub struct UpdateCheckRun {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub conclusion: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub started_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output: Option<Output>,
}

#[derive(Serialize, Deserialize, Clone, Copy)]
pub struct Empty {}
