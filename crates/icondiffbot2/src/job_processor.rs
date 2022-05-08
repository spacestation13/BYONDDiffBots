use std::path::Path;

use anyhow::{format_err, Result};
use diffbot_lib::{
    github::{
        github_api::download_url,
        github_types::{CheckOutputBuilder, CheckOutputs, ModifiedFileStatus},
    },
    job::types::Job,
};
use dmm_tools::dmi::{Dir, IconFile, Image};
use tokio::runtime::Handle;

use crate::CONFIG;

pub fn do_job(job: &Job) -> Result<CheckOutputs> {
    // TODO: Maybe have jobs just be async?
    let handle = Handle::try_current()?;
    handle.block_on(async { handle_changed_files(job).await })
}

fn status_to_sha(job: &Job, status: ModifiedFileStatus) -> (Option<&str>, Option<&str>) {
    match status {
        ModifiedFileStatus::Added => (None, Some(&job.head.sha)),
        ModifiedFileStatus::Removed => (Some(&job.base.sha), None),
        ModifiedFileStatus::Modified => (Some(&job.base.sha), Some(&job.head.sha)),
        ModifiedFileStatus::Renamed => (None, None),
        ModifiedFileStatus::Copied => (None, None),
        ModifiedFileStatus::Changed => (None, None), // TODO: look up what this is
        ModifiedFileStatus::Unchanged => (None, None),
    }
}

struct IconFileWithName {
    pub name: String,
    pub sha: String,
    pub icon: IconFile,
}

async fn get_if_exists(job: &Job, filename: &str, sha: Option<&str>) -> Option<IconFileWithName> {
    if let Some(sha) = sha {
        Some(IconFileWithName {
            name: filename.to_string(),
            sha: sha.to_string(),
            icon: IconFile::from_raw(
                download_url(&job.installation, &job.base.repo, filename, sha)
                    .await
                    .ok()?,
            )
            .ok()?,
        })
    } else {
        None
    }
}

async fn sha_to_iconfile(
    job: &Job,
    filename: &str,
    sha: (Option<&str>, Option<&str>),
) -> (Option<IconFileWithName>, Option<IconFileWithName>) {
    (
        get_if_exists(job, filename, sha.0).await,
        get_if_exists(job, filename, sha.1).await,
    )
}

pub async fn handle_changed_files(job: &Job) -> Result<CheckOutputs> {
    job.check_run.mark_started().await.unwrap();

    let mut output_builder =
        CheckOutputBuilder::new("Icon difference rendering", "Omegalul pog pog pog");

    for dmi in &job.files {
        output_builder.add_text(
            &render(sha_to_iconfile(job, &dmi.filename, status_to_sha(job, dmi.status)).await)
                .await
                .unwrap_or_else(|e| format!("Error: {e}")),
        );
    }

    Ok(output_builder.build())
}

async fn render(diff: (Option<IconFileWithName>, Option<IconFileWithName>)) -> Result<String> {
    if diff.0.is_some() || diff.1.is_none() {
        todo!()
    }

    let after = diff.1.unwrap();
    let after_icon = after.icon;

    dbg!(&after_icon.metadata);

    let mut canvas = Image::new_rgba(after_icon.metadata.width, after_icon.metadata.height);
    let no_tint = [0xff, 0xff, 0xff, 0xff];

    let blank = canvas.data.clone();

    let mut builder = String::new();

    for state in &after_icon.metadata.states {
        canvas.composite(
            &after_icon.image,
            (0, 0),
            after_icon
                .rect_of(&state.name, Dir::South)
                .ok_or_else(|| format_err!("Failed to get icon_state {}", &state.name))?,
            no_tint,
        );
        let filename = format!("{}-{}-{}.png", &after.sha, &after.name, &state.name);
        canvas
            .to_file(&Path::new(".").join("images").join(&filename))
            .unwrap();
        // TODO: tempted to use an <img> tag so i can set a style that upscales 32x32 to 64x64 and sets all the browser flags for nearest neighbor scaling
        let url = format!("{}/{}", CONFIG.get().unwrap().file_hosting_url, filename);
        builder.push_str(&format!(
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/templates/diff_line.txt"
            )),
            state_name = state.name,
            old = "",
            new = url,
        ));
        builder.push('\n');
        canvas.data = blank.clone();
    }

    Ok(format!(
        include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/templates/diff_add.txt"
        )),
        filename = after.name,
        table = builder
    ))
}
