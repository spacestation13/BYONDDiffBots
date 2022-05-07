use std::path::PathBuf;

use rocket::tokio::fs::File;
use rocket::tokio::io::AsyncWriteExt;

use crate::error::Error;
use crate::github_types::{Installation, ModifiedFile, PullRequest, Repository};

static DOWNLOAD_DIR: &str = "download";

pub async fn download_file<S: AsRef<str>>(
    installation: u64,
    repo: &Repository,
    filename: S,
    commit: S,
) -> Result<PathBuf, Error> {
    let (owner, repo) = repo.full_name();
    let items = octocrab::instance()
        .installation(installation.into())
        .repos(owner, repo)
        .get_content()
        .path(filename.as_ref())
        .r#ref(commit.as_ref())
        .send()
        .await?
        .take_items();

    if items.len() > 1 {
        return Err(Error::DirectoryGivenToDownloadFile);
    }

    let target = &items[0];

    let mut path = PathBuf::new();
    path.push(".");
    path.push(DOWNLOAD_DIR);
    path.push(&target.sha);
    path.set_extension("dmi");

    rocket::tokio::fs::create_dir_all(path.parent().unwrap()).await?;
    let mut file = File::create(&path).await?;

    let download_url = target.download_url.as_ref().ok_or(Error::NoDownloadUrl)?;

    let response = reqwest::get(download_url).await?;

    file.write_all(&response.bytes().await?).await?;

    Ok(path)
}

pub async fn get_pull_files(
    installation: &Installation,
    pull: &PullRequest,
) -> Result<Vec<ModifiedFile>, String> {
    let (owner, repo) = pull.base.repo.full_name();
    let res = octocrab::instance()
        .installation(installation.id.into())
        .get(
            &format!(
                "/repos/{user}/{repo}/pulls/{pull_number}/files",
                user = owner,
                repo = repo,
                pull_number = pull.number
            ),
            None::<&()>,
        )
        .await
        .map_err(|e| format!("{e}"))?;

    Ok(res)
}
