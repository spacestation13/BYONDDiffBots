use std::{collections::BTreeMap, path::Path, sync::Arc};

use anyhow::{format_err, Result};
use diffbot_lib::{
    github::{
        github_api::download_url,
        github_types::{CheckOutputBuilder, CheckOutputs, ModifiedFileStatus},
    },
    job::types::Job,
};
use dmm_tools::dmi::render::IconRenderer;
use dmm_tools::dmi::IconFile;
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
            .unwrap(), // .unwrap_or_else(|e| format!("Error: {e}")),
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
            let mut seen_names: BTreeMap<String, u32> = BTreeMap::new();
            for url in urls {
                let mut state_name = url.0;
                // Mark default states
                if state_name.is_empty() {
                    state_name = "{{DEFAULT}}".to_string();
                }

                // Deduplicate state names
                if let Some(value) = seen_names.get_mut(&state_name) {
                    *value += 1;
                    state_name = format!("{state_name}{value}");
                } else {
                    seen_names.insert(state_name.clone(), 1);
                }

                builder.push_str(&format!(
                    include_str!(concat!(
                        env!("CARGO_MANIFEST_DIR"),
                        "/templates/diff_line.txt"
                    )),
                    state_name = state_name,
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
            let mut seen_names: BTreeMap<String, u32> = BTreeMap::new();
            for url in urls {
                let mut state_name = url.0;
                // Mark default states
                if state_name.is_empty() {
                    state_name = "{{DEFAULT}}".to_string();
                }

                // Deduplicate state names
                if let Some(value) = seen_names.get_mut(&state_name) {
                    *value += 1;
                    state_name = format!("{state_name}{value}");
                } else {
                    seen_names.insert(state_name.clone(), 1);
                }

                // Build the output line
                builder.push_str(&format!(
                    include_str!(concat!(
                        env!("CARGO_MANIFEST_DIR"),
                        "/templates/diff_line.txt"
                    )),
                    state_name = state_name,
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
    let icon = &target.icon;

    let mut vec = Vec::new();

    let mut renderer = IconRenderer::new(icon);

    for (state_no, state) in icon.metadata.states.iter().enumerate() {
        let access = job.lock().await;
        let prefix = format!("{}/{}", access.installation, access.pull_request);
        let directory = Path::new(".").join("images").join(&prefix);
        // Always remember to mkdir -p your paths
        std::fs::create_dir_all(&directory)?;
        drop(access);
        let filename = format!(
            "{}-{}-{}-{}",
            // Differentiate between before-after files
            &target.sha,
            // Differentiate between different files in the same commit
            &target.name.replace(".dmi", ""),
            // Differentiate between duplicate states
            state_no,
            // Diffentiate between states.
            sanitize_filename::sanitize(&state.name)
        );

        let path = directory.join(&filename);
        // dbg!(&path, &state.frames);
        let corrected_path = renderer.render_state(state, path)?;
        let extension = corrected_path
            .extension()
            .ok_or_else(|| format_err!("Unable to get extension that was written to"))?;
        // dbg!(&corrected_path, &extension);

        vec.push((
            state.name.clone(),
            format!(
                "{}/{}/{}.{}",
                CONFIG.get().unwrap().file_hosting_url,
                prefix,
                filename,
                extension.to_string_lossy()
            ),
        ));
    }

    dbg!(&vec);

    Ok(vec)
}
