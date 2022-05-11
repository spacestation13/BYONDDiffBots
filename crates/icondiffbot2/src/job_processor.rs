use std::{path::Path, sync::Arc};

use anyhow::Result;
use diffbot_lib::{
    github::{
        github_api::download_url,
        github_types::{CheckOutputBuilder, CheckOutputs, ModifiedFileStatus},
    },
    job::types::Job,
};
use dmm_tools::dmi::{Dir, IconFile, Image, State};
use tokio::{runtime::Handle, sync::Mutex};

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

    let protected_job = Arc::new(Mutex::new(job));

    for dmi in &job.files {
        output_builder.add_text(
            &render(
                Arc::clone(&protected_job),
                sha_to_iconfile(job, &dmi.filename, status_to_sha(job, dmi.status)).await,
            )
            .await
            .unwrap_or_else(|e| format!("Error: {e}")),
        );
    }

    Ok(output_builder.build())
}

async fn render(
    job: Arc<Mutex<&Job>>,
    diff: (Option<IconFileWithName>, Option<IconFileWithName>),
) -> Result<String> {
    // TODO: Generate gifs from animation frames
    // TODO: Tile directions from left to right
    // TODO: Don't blindly render to images/ directory
    // TODO: Table should be |Icon State|Old|New|Changes|
    // TODO: Sanitize icon_state to be filesystem safe
    match diff {
        (None, None) => Ok("".to_string()),
        (None, Some(after)) => {
            let urls = full_render(job, &after).await?;
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
            let urls = full_render(job, &before).await?;
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
        (Some(_before), Some(_after)) => {
            todo!()
        }
    }
}

async fn full_render(
    job: Arc<Mutex<&Job>>,
    target: &IconFileWithName,
) -> Result<Vec<(String, String)>> {
    let after_icon = &target.icon;

    let mut vec = Vec::new();

    for state in &after_icon.metadata.states {
        let access = job.lock().await;
        let prefix = format!("{}/{}", access.installation, access.pull_request);
        drop(access);
        let filename = format!(
            "{}/{}-{}-{}",
            prefix, &target.sha, &target.name, &state.name
        );
        let filename = render_state(state, after_icon, &filename).await?;

        // let path = Path::new(".").join("images").join(&filename);
        // tokio::fs::create_dir_all(&path.parent().unwrap()).await?;
        // canvas.to_file(&path).unwrap();

        vec.push((
            state.name.clone(),
            format!("{}/{}", CONFIG.get().unwrap().file_hosting_url, filename),
        ));
    }

    Ok(vec)
}

async fn render_state(state: &State, icon: &IconFile, filename: &str) -> Result<String> {
    let renders = match state.dirs {
        dmm_tools::dmi::Dirs::One => [icon.render(&state.name, Dir::South)?].to_vec(),
        dmm_tools::dmi::Dirs::Four => [
            icon.render(&state.name, Dir::South)?,
            icon.render(&state.name, Dir::North)?,
            icon.render(&state.name, Dir::East)?,
            icon.render(&state.name, Dir::West)?,
        ]
        .to_vec(),
        dmm_tools::dmi::Dirs::Eight => [
            icon.render(&state.name, Dir::South)?,
            icon.render(&state.name, Dir::North)?,
            icon.render(&state.name, Dir::East)?,
            icon.render(&state.name, Dir::West)?,
            icon.render(&state.name, Dir::Northeast)?,
            icon.render(&state.name, Dir::Northwest)?,
            icon.render(&state.name, Dir::Southeast)?,
            icon.render(&state.name, Dir::Southwest)?,
        ]
        .to_vec(),
    };

    let first_dir = renders.get(0).unwrap();
    let first_frame = first_dir.frames.get(0).unwrap();

    let frames: Vec<Image> = if renders.len() > 1 {
        (0..first_dir.frames.len())
            .map(|frame| {
                let mut canvas = Image::new_rgba(
                    first_frame.width * (renders.len() as u32),
                    first_frame.height,
                );
                renders.iter().enumerate().for_each(|(dir_no, dir)| {
                    let dir_frame = dir.frames.get(frame).unwrap();
                    let crop = (0, 0, dir_frame.width, dir_frame.height);
                    let no_tint = [0xff, 0xff, 0xff, 0xff];
                    canvas.composite(
                        dir_frame,
                        (first_frame.width * (dir_no as u32), 0),
                        crop,
                        no_tint,
                    );
                });
                canvas
            })
            .collect()
    } else {
        // gotta go fast
        first_dir.frames.clone()
    };
    let first_frame = frames.get(0).unwrap();

    let filename = format!("{}.gif", filename);
    let path = Path::new(".").join("images").join(&filename);
    tokio::fs::create_dir_all(&path.parent().unwrap()).await?;

    let mut new_render_result = first_dir.clone();
    new_render_result.size = (first_frame.width, first_frame.height);
    new_render_result.frames = frames;

    // TODO: produce png for unanimated
    IconFile::write_gif(std::fs::File::create(path)?, &new_render_result)?;
    Ok(filename)
}
