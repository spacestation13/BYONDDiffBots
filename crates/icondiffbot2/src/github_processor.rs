use std::path::PathBuf;

use diffbot_lib::{
    github::github_api::{download_file, get_pull_files},
    github::{
        github_api::CheckRun,
        github_types::{self, ModifiedFile, PullRequestEventPayload},
    },
};
use dmm_tools::dmi::{Dir, IconFile, Image};
// use dmm_tools::dmi::IconFile;
use rocket::{
    http::Status,
    post,
    request::{FromRequest, Outcome},
    Request,
};

#[derive(Debug)]
pub struct GithubEvent(pub String);

#[rocket::async_trait]
impl<'r> FromRequest<'r> for GithubEvent {
    type Error = &'static str;

    async fn from_request(req: &'r Request<'_>) -> Outcome<Self, Self::Error> {
        match req.headers().get_one("X-Github-Event") {
            Some(event) => Outcome::Success(GithubEvent(event.to_owned())),
            None => Outcome::Failure((Status::BadRequest, "Missing X-Github-Event header")),
        }
    }
}

#[post("/payload", format = "json", data = "<payload>")]
pub async fn process_github_payload(
    event: GithubEvent,
    payload: String,
) -> Result<&'static str, String> {
    if event.0 != "pull_request" {
        return Ok("Not a pull request event");
    }

    let payload: PullRequestEventPayload =
        serde_json::from_str(&payload).map_err(|e| format!("{e}"))?;

    if payload.action != "opened" && payload.action != "reopened" && payload.action != "synchronize"
    {
        return Ok("PR not opened or updated");
    }

    let files = get_pull_files(&payload.installation, &payload.pull_request)
        .await
        .map_err(|e| format!("{e}"))?;

    let changed_dmis: Vec<ModifiedFile> = files
        .into_iter()
        .filter(|e| e.filename.ends_with(".dmi"))
        .collect();

    if changed_dmis.is_empty() {
        return Ok("");
    }

    rocket::tokio::spawn(handle_changed_files(payload, changed_dmis));

    Ok("")
}

pub async fn handle_changed_files(
    payload: PullRequestEventPayload,
    changed_dmis: Vec<ModifiedFile>,
) {
    let check_run = CheckRun::create(
        &payload.pull_request.base.repo.full_name(),
        &payload.pull_request.head.sha,
        payload.installation.id,
        Some("IconDiffBot2"),
    )
    .await
    .unwrap();

    check_run.mark_started().await.unwrap();

    for dmi in changed_dmis {
        match dmi.status {
            github_types::ModifiedFileStatus::Added => {
                let new = download_file(
                    payload.installation.id,
                    &payload.repository,
                    &dmi.filename,
                    &payload.pull_request.head.sha,
                )
                .await
                .unwrap();

                let new = read_icon_file(new).await;
                render(None, Some(new)).await;
            }
            github_types::ModifiedFileStatus::Removed => {
                let old = download_file(
                    payload.installation.id,
                    &payload.repository,
                    &dmi.filename,
                    &payload.pull_request.base.sha,
                )
                .await
                .unwrap();

                dbg!(&old);

                let old = read_icon_file(old).await;
                render(Some(old), None).await;
            }
            github_types::ModifiedFileStatus::Modified => {
                let old = download_file(
                    payload.installation.id,
                    &payload.repository,
                    &dmi.filename,
                    &payload.pull_request.base.sha,
                )
                .await
                .unwrap();
                let new = download_file(
                    payload.installation.id,
                    &payload.repository,
                    &dmi.filename,
                    &payload.pull_request.head.sha,
                )
                .await
                .unwrap();

                dbg!(&old, &new);

                let old = read_icon_file(old).await;
                let new = read_icon_file(new).await;

                render(Some(old), Some(new)).await;
            }
            github_types::ModifiedFileStatus::Renamed => todo!(),
            github_types::ModifiedFileStatus::Copied => todo!(),
            github_types::ModifiedFileStatus::Changed => todo!(),
            github_types::ModifiedFileStatus::Unchanged => todo!(),
        }
    }

    check_run
        .mark_failed("Not implemented yet lol get rekt nerd")
        .await
        .unwrap();
}

/// Helper to prevent files lasting longer than needed
/// TODO: Remove when FileGuard/In Memory Only is set up
async fn read_icon_file(path: PathBuf) -> IconFile {
    let file = IconFile::from_file(&path).unwrap();
    rocket::tokio::fs::remove_file(path).await.unwrap();
    file
}

async fn render(before: Option<IconFile>, after: Option<IconFile>) {
    if before.is_some() || after.is_none() {
        todo!()
    }

    let after = after.unwrap();

    dbg!(&after.metadata);

    let mut canvas = Image::new_rgba(after.metadata.width, after.metadata.height);

    canvas.composite(
        &after.image,
        (0, 0),
        after.rect_of("hot_dispenser", Dir::South).unwrap(),
        [0xff, 0xff, 0xff, 0xff],
    );

    canvas.to_file(&PathBuf::from("test.png"));
}
