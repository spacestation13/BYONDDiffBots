use derive_builder::Builder;
use serde::{Deserialize, Serialize};

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
    pub fn full_name(&self) -> String {
        self.url.split('/').skip(4).collect::<Vec<&str>>().join("/")
    }

    pub fn name_tuple(&self) -> (String, String) {
        let mut iter = self.url.split('/').skip(4).take(2).map(|a| a.to_string());
        (iter.next().unwrap(), iter.next().unwrap())
    }

    // pub fn owner(&self) -> String {
    //     self.url
    //         .split('/')
    //         .skip(4)
    //         .take(1)
    //         .collect::<Vec<&str>>()
    //         .join("")
    // }
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

#[derive(Serialize, Debug)]
pub struct Output {
    pub title: String,
    pub summary: String,
    pub text: String,
}

#[derive(Serialize)]
pub struct CreateCheckRun {
    pub name: String,
    pub head_sha: String,
}

#[derive(Serialize, Builder, Default)]
#[builder(pattern = "owned")]
#[builder(default)]
#[builder(setter(into, strip_option))]
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

#[derive(Debug)]
pub enum CheckOutputs {
    One(Output),
    Many(Output, Vec<Output>),
}

#[derive(Debug)]
pub struct CheckOutputBuilder {
    title: String,
    summary: String,
    current_text: String,
    outputs: Vec<Output>,
}

impl CheckOutputBuilder {
    pub fn new<S: Into<String>>(title: S, summary: S) -> Self {
        let title = title.into();
        let summary = summary.into();
        Self {
            title,
            summary,
            current_text: String::new(),
            outputs: Vec::new(),
        }
    }

    pub fn add_text(&mut self, text: &str) {
        self.current_text.push_str(text);
        // Leaving a 5k character safety margin is prob overkill but oh well
        if self.current_text.len() > 60_000 {
            let output = Output {
                title: self.title.clone(),
                summary: self.summary.clone(),
                text: std::mem::take(&mut self.current_text),
            };
            self.outputs.push(output);
        }
    }

    pub fn build(self) -> CheckOutputs {
        let Self {
            title,
            summary,
            current_text,
            mut outputs,
        } = self;

        if !current_text.is_empty() {
            let output = Output {
                title,
                summary,
                text: current_text,
            };
            outputs.push(output);
        }
        let first = outputs.remove(0);
        if outputs.is_empty() {
            CheckOutputs::One(first)
        } else {
            CheckOutputs::Many(first, outputs)
        }
    }
}
