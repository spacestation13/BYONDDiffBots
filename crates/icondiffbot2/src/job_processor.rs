use std::path::PathBuf;

use anyhow::{format_err, Result};
use diffbot_lib::{
    github::{
        github_api::download_file,
        github_types::{CheckOutputs, ModifiedFileStatus},
    },
    job::types::Job,
};
use dmm_tools::dmi::{Dir, IconFile, Image};
use tokio::runtime::Handle;

pub fn do_job(job: &Job) -> Result<CheckOutputs> {
    // TODO: Maybe have jobs just be async?
    let handle = Handle::try_current()?;
    handle.block_on(async { handle_changed_files(job).await })
}

pub async fn handle_changed_files(job: &Job) -> Result<CheckOutputs> {
    job.check_run.mark_started().await.unwrap();

    for dmi in &job.files {
        match dmi.status {
            ModifiedFileStatus::Added => {
                let new = download_file(
                    &job.installation,
                    &job.head.repo,
                    &dmi.filename,
                    &job.head.sha,
                )
                .await
                .unwrap();

                let new = read_icon_file(new).await;
                render(None, Some(new)).await;
            }
            ModifiedFileStatus::Removed => {
                let old = download_file(
                    &job.installation,
                    &job.base.repo,
                    &dmi.filename,
                    &job.base.sha,
                )
                .await
                .unwrap();

                dbg!(&old);

                let old = read_icon_file(old).await;
                render(Some(old), None).await;
            }
            ModifiedFileStatus::Modified => {
                let old = download_file(
                    &job.installation,
                    &job.base.repo,
                    &dmi.filename,
                    &job.base.sha,
                )
                .await
                .unwrap();
                let new = download_file(
                    &job.installation,
                    &job.head.repo,
                    &dmi.filename,
                    &job.head.sha,
                )
                .await
                .unwrap();

                dbg!(&old, &new);

                let old = read_icon_file(old).await;
                let new = read_icon_file(new).await;

                render(Some(old), Some(new)).await;
            }
            ModifiedFileStatus::Renamed => todo!(),
            ModifiedFileStatus::Copied => todo!(),
            ModifiedFileStatus::Changed => todo!(),
            ModifiedFileStatus::Unchanged => todo!(),
        }
    }

    Err(format_err!("Unimplemented"))
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

    canvas.to_file(&PathBuf::from("test.png")).unwrap();
}
