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
    // TODO: Generate gifs from animation frames
    // TODO: Tile directions from left to right
    // TODO: Don't blindly render to images/ directory
    // TODO: Table should be |Icon State|Old|New|Changes|
    // TODO: Sanitize icon_state to be filesystem safe
    match diff {
        (None, None) => Ok("".to_string()),
        (None, Some(after)) => {
            let urls = full_render(&after).await?;
            // TODO: tempted to use an <img> tag so i can set a style that upscales 32x32 to 64x64 and sets all the browser flags for nearest neighbor scaling
            let mut builder = String::new();
            for url in urls {
                builder.push_str(&format!(
                    include_str!(concat!(
                        env!("CARGO_MANIFEST_DIR"),
                        "/templates/diff_line.txt"
                    )),
                    state_name = url.0,
                    old = "",
                    new = url.1,
                ));
                builder.push('\n');
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
        (Some(before), None) => {
            dbg!(&before.icon.metadata);
            let urls = full_render(&before).await?;
            dbg!(&urls);
            // TODO: tempted to use an <img> tag so i can set a style that upscales 32x32 to 64x64 and sets all the browser flags for nearest neighbor scaling
            let mut builder = String::new();
            for url in urls {
                builder.push_str(&format!(
                    include_str!(concat!(
                        env!("CARGO_MANIFEST_DIR"),
                        "/templates/diff_line.txt"
                    )),
                    state_name = url.0,
                    old = url.1,
                    new = "",
                ));
                builder.push('\n');
            }

            Ok(format!(
                include_str!(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/templates/diff_remove.txt"
                )),
                filename = before.name,
                table = builder
            ))
        }
        (Some(before), Some(after)) => {
            todo!()
        }
    }
}

async fn full_render(target: &IconFileWithName) -> Result<Vec<(String, String)>> {
    let after_icon = &target.icon;

    let mut canvas = Image::new_rgba(after_icon.metadata.width, after_icon.metadata.height);
    let no_tint = [0xff, 0xff, 0xff, 0xff];
    let blank = canvas.data.clone();

    let mut vec = Vec::new();

    for state in &after_icon.metadata.states {
        canvas.composite(
            &after_icon.image,
            (0, 0),
            after_icon
                .rect_of(&state.name, Dir::South)
                .ok_or_else(|| format_err!("Failed to get icon_state {}", &state.name))?,
            no_tint,
        );
        let filename = format!("{}-{}-{}.png", &target.sha, &target.name, &state.name);
        canvas
            .to_file(&Path::new(".").join("images").join(&filename))
            .unwrap();

        vec.push((
            state.name.clone(),
            format!("{}/{}", CONFIG.get().unwrap().file_hosting_url, filename),
        ));
        canvas.data = blank.clone();
    }

    Ok(vec)
}
