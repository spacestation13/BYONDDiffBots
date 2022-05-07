use std::path::PathBuf;

use crate::{
    github_api::download_file,
    github_types::{ModifiedFile, PullRequestEventPayload},
};
use dmm_tools::dmi::{Dir, IconFile, Image};
// use dmm_tools::dmi::IconFile;
use rocket::{
    http::Status,
    post,
    request::{FromRequest, Outcome},
    Request,
};

use crate::github_api::get_pull_files;

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

    let files = get_pull_files(&payload.installation, &payload.pull_request).await?;

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
    for dmi in changed_dmis {
        match dmi.status {
            crate::github_types::ModifiedFileStatus::Added => {
                let new = download_file(
                    payload.installation.id,
                    &payload.repository,
                    &dmi.filename,
                    &payload.pull_request.head.sha,
                )
                .await
                .unwrap();

                let file = IconFile::from_file(&new).unwrap();

                let mut canvas = Image::new_rgba(file.metadata.width, file.metadata.height);

                canvas.composite(
                    &file.image,
                    (0, 0),
                    file.rect_of("hot_dispenser", Dir::South).unwrap(),
                    [0xff, 0xff, 0xff, 0xff],
                );

                canvas.to_file(&PathBuf::from("test.png"));

                dbg!(&file.metadata);

                rocket::tokio::fs::remove_file(new).await.unwrap();
            }
            crate::github_types::ModifiedFileStatus::Removed => todo!(),
            crate::github_types::ModifiedFileStatus::Modified => {
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

                rocket::tokio::fs::remove_file(old).await.unwrap();
                rocket::tokio::fs::remove_file(new).await.unwrap();
            }
            crate::github_types::ModifiedFileStatus::Renamed => todo!(),
            crate::github_types::ModifiedFileStatus::Copied => todo!(),
            crate::github_types::ModifiedFileStatus::Changed => todo!(),
            crate::github_types::ModifiedFileStatus::Unchanged => todo!(),
        }
    }
}
