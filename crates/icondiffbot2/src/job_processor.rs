use std::path::PathBuf;

use anyhow::{format_err, Result};
use diffbot_lib::{
    github::{
        github_api::download_file,
        github_types::{CheckOutputBuilder, CheckOutputs, ModifiedFileStatus},
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

fn status_to_sha(job: &Job, status: ModifiedFileStatus) -> (Option<String>, Option<String>) {
    match status {
        ModifiedFileStatus::Added => (None, Some(job.head.sha.clone())),
        ModifiedFileStatus::Removed => (Some(job.base.sha.clone()), None),
        ModifiedFileStatus::Modified => (Some(job.base.sha.clone()), Some(job.head.sha.clone())),
        ModifiedFileStatus::Renamed => (None, None),
        ModifiedFileStatus::Copied => (None, None),
        ModifiedFileStatus::Changed => (None, None), // TODO: look up what this is
        ModifiedFileStatus::Unchanged => (None, None),
    }
}
// let new = download_file(
//     &job.installation,
//     &job.head.repo,
//     &dmi.filename,
//     &job.head.sha,
// )
// .await
// .unwrap();

pub async fn handle_changed_files(job: &Job) -> Result<CheckOutputs> {
    job.check_run.mark_started().await.unwrap();

    let output_builder =
        CheckOutputBuilder::new("Icon difference rendering", "Omegalul pog pog pog");

    for dmi in &job.files {
        match status_to_sha(job, dmi.status) {
            (None, None) => todo!(),
            (None, Some(new)) => todo!(),
            (Some(old), None) => todo!(),
            (Some(old), Some(new)) => todo!(),
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
