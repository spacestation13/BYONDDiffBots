use crate::github::github_types::{
    CreateCheckRun, Output, RawCheckRun, Repository, UpdateCheckRunBuilder,
};
use eyre::{format_err, Context, Result};
use octocrab::models::repos::Content;
use octocrab::models::InstallationId;
use serde::{Deserialize, Serialize};
use std::{future::Future, pin::Pin};
use base64::{Engine as _, engine::general_purpose};

pub struct GithubEvent(pub String, pub Option<Vec<u8>>);

impl actix_web::FromRequest for GithubEvent {
    type Error = std::io::Error;

    type Future = Pin<Box<dyn Future<Output = Result<Self, Self::Error>>>>;

    fn from_request(req: &actix_web::HttpRequest, _: &mut actix_web::dev::Payload) -> Self::Future {
        let req = req.clone();
        Box::pin(async move {
            let event_header = match req.headers().get("X-Github-Event") {
                Some(event) => event
                    .to_str()
                    .map_err(|_| {
                        std::io::Error::new(
                            std::io::ErrorKind::InvalidData,
                            "Corrupt X-Github-Event header, failed to convert to string",
                        )
                    })?
                    .to_owned(),
                None => {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        "Missing X-Github-Event header",
                    ))
                }
            };
            let hmac_header = match req.headers().get("X-Hub-Signature-256") {
                Some(event) => {
                    let sig = event.to_str().map_err(|_| {
                        std::io::Error::new(
                            std::io::ErrorKind::InvalidData,
                            "Corrupt X-Hub-Signature-256 header, failed to convert to string",
                        )
                    })?;

                    //remove the `sha256=` part
                    let (_, sig) = sig.split_at(7);

                    let sig_bytes = hex::decode(sig).map_err(|_| {
                        std::io::Error::new(
                            std::io::ErrorKind::InvalidData,
                            "Corrupt X-Hub-Signature-256 header, failed to decode hex string",
                        )
                    })?;

                    Some(sig_bytes)
                }
                _ => None,
            };
            Ok(GithubEvent(event_header, hmac_header))
        })
    }
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct CheckRun {
    id: u64,
    installation_id: InstallationId,
    head_sha: String,
    repo: String,
}

impl CheckRun {
    pub async fn create<I: Into<InstallationId>>(
        full_repo: &str,
        head_sha: &str,
        inst_id: I,
        name: Option<&str>,
    ) -> Result<Self> {
        let inst_id = inst_id.into();
        let result: RawCheckRun = octocrab::instance()
            .installation(inst_id)
            .post(
                format!("/repos/{full_repo}/check-runs"),
                Some(&CreateCheckRun {
                    name: name.unwrap_or("BYONDDiffBot").to_string(),
                    head_sha: head_sha.to_string(),
                }),
            )
            .await
            .wrap_err("Submitting check")?;

        Ok(Self {
            id: result.id,
            installation_id: inst_id,
            head_sha: head_sha.to_string(),
            repo: full_repo.to_owned(),
        })
    }

    /// Creates a new check run for the same PR
    pub async fn duplicate(&self, name: &str) -> Result<Self> {
        Self::create(&self.repo, &self.head_sha, self.installation_id, Some(name)).await
    }

    pub async fn rename(&self, name: &str) -> Result<()> {
        self.update(UpdateCheckRunBuilder::default().name(name.to_owned()))
            .await
            .wrap_err("Renaming check run")
    }

    pub async fn mark_queued(&self) -> Result<()> {
        self.update(
            UpdateCheckRunBuilder::default()
                .status("queued")
                .started_at(chrono::Utc::now().to_rfc3339()),
        )
        .await
        .wrap_err("Marking check run as queued")
    }

    pub async fn mark_started(&self) -> Result<()> {
        self.update(
            UpdateCheckRunBuilder::default()
                .status("in_progress")
                .started_at(chrono::Utc::now().to_rfc3339()),
        )
        .await
        .wrap_err("Marking check run as in progress")
    }

    pub async fn mark_failed(&self, stack_trace: &str) -> Result<()> {
        let summary = format!(
            include_str!("error_template.txt"),
            stack_trace = stack_trace
        );

        self.update(
            UpdateCheckRunBuilder::default()
                .status("completed")
                .conclusion("failure")
                .completed_at(chrono::Utc::now().to_rfc3339())
                .output(Output {
                    title: "Error handling job",
                    summary,
                    text: "".to_owned(),
                }),
        )
        .await
        .wrap_err("Marking check as failure")
    }

    pub async fn mark_succeeded(&self, output: Output) -> Result<()> {
        self.update(
            UpdateCheckRunBuilder::default()
                .conclusion("success")
                .completed_at(chrono::Utc::now().to_rfc3339())
                .output(output),
        )
        .await
        .wrap_err("Marking check as success")
    }

    pub async fn mark_skipped(&self, output: Output) -> Result<()> {
        self.update(
            UpdateCheckRunBuilder::default()
                .conclusion("skipped")
                .completed_at(chrono::Utc::now().to_rfc3339())
                .output(output),
        )
        .await
        .wrap_err("Marking check as skipped")
    }

    pub async fn set_output(&self, output: Output) -> Result<()> {
        self.update(UpdateCheckRunBuilder::default().output(output))
            .await
            .wrap_err("Setting check run output")
    }

    async fn update(&self, builder: UpdateCheckRunBuilder) -> Result<()> {
        let update = builder.build().wrap_err("Building UpdateCheckRun")?;

        #[derive(Deserialize)]
        struct Empty {}
        let _: Empty = octocrab::instance()
            .installation(self.installation_id)
            .patch(
                format!(
                    "/repos/{repo}/check-runs/{check_run_id}",
                    repo = self.repo,
                    check_run_id = self.id,
                ),
                Some(&update),
            )
            .await
            .wrap_err("Updating check run")?;

        Ok(())
    }

    pub fn id(&self) -> u64 {
        self.id
    }
}

async fn find_content<S: AsRef<str>>(
    installation: &InstallationId,
    repo: &Repository,
    filename: S,
    commit: S,
) -> Result<Content> {
    let (owner, repo) = repo.name_tuple();
    let items = octocrab::instance()
        .installation(*installation)
        .repos(owner, repo)
        .get_content()
        .path(filename.as_ref())
        .r#ref(commit.as_ref())
        .send()
        .await?
        .take_items();

    if items.len() > 1 {
        return Err(format_err!("Directory given to find_content"));
    }

    items
        .into_iter()
        .next()
        .ok_or_else(|| format_err!("No content was found"))
}

pub async fn download_url<S: AsRef<str>>(
    installation: &InstallationId,
    repo: &Repository,
    filename: S,
    commit: S,
) -> Result<Vec<u8>> {
    let target = find_content(installation, repo, filename, commit).await?;

    let content = target.content
        .ok_or_else(|| format_err!("File had no content!"))?
        .replace("\n", "");

    general_purpose::STANDARD
        .decode(content)
        .map_err(|decode_error| format_err!("DecodeError: {}", decode_error))
}

/* local test requires commenting out the .installation(...) call in find_content(), a valid github token with access, and the following dep: actix-rt = "2.9.0"

#[actix_web::rt::test]
async fn test_private_repo_file_download() {
    octocrab::initialise(octocrab::OctocrabBuilder::new()
        .personal_token("lol".to_owned())
        .build()
        .unwrap());

    let bytes = download_url(
        &InstallationId(0),
        &Repository{
        url: "https://api.github.com/repos/Cyberboss/tgstation-private-test".to_owned(),
        id:0,
    }, ".tgs.yml", "140c79189849ea616f09b3484f8930211d3705cd").await.unwrap();

    let text = std::str::from_utf8(bytes.as_slice()).unwrap();
    assert_eq!(r#"# This file is used by TGS (https://github.com/tgstation/tgstation-server) clients to quickly initialize a server instance for the codebase
# The format isn't documented anywhere but hopefully we never have to change it. If there are questions, contact the TGS maintainer Cyberboss/@Dominion#0444
version: 1
# The BYOND version to use (kept in sync with dependencies.sh by the "TGS Test Suite" CI job)
# Must be interpreted as a string, keep quoted
byond: "514.1588"
# Folders to create in "<instance_path>/Configuration/GameStaticFiles/"
static_files:
  # Config directory should be static
  - name: config
    # This implies the folder should be pre-populated with contents from the repo
    populate: true
  # Data directory must be static
  - name: data
# String dictionary. The value is the location of the file in the repo to upload to TGS. The key is the name of the file to upload to "<instance_path>/Configuration/EventScripts/"
# This one is for Linux hosted servers
linux_scripts:
  PreCompile.sh: tools/tgs_scripts/PreCompile.sh
  WatchdogLaunch.sh: tools/tgs_scripts/WatchdogLaunch.sh
  InstallDeps.sh: tools/tgs_scripts/InstallDeps.sh
# Same as above for Windows hosted servers
windows_scripts:
  PreCompile.bat: tools/tgs_scripts/PreCompile.bat
# The security level the game should be run at
security: Trusted
"#, text)
}
*/
