// should fix but lazy
#![allow(clippy::format_push_string)]

use std::{collections::HashSet, path::Path, sync::Arc};

use anyhow::{format_err, Context, Result};
use diffbot_lib::{
    github::{
        github_api::download_url,
        github_types::{CheckOutputBuilder, CheckOutputs, ModifiedFileStatus},
    },
    job::types::Job,
};
use dmm_tools::dmi::render::IconRenderer;
use dmm_tools::dmi::{IconFile, State};
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

    // TODO: not omegalul
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
            .unwrap_or_else(|e| format!("Error: {:?}", e)),
        );
    }

    Ok(output_builder.build())
}

async fn render(
    job: Arc<Mutex<&Job>>,
    diff: (Option<IconFileWithName>, Option<IconFileWithName>),
) -> Result<String> {
    // TODO: Alphabetize
    // TODO: Test more edge cases
    // TODO: Parallelize?
    match diff {
        (None, None) => Ok("".to_string()),
        (None, Some(after)) => {
            let urls = full_render(job, &after)
                .await
                .context("Failed to render new icon file")?;
            // TODO: tempted to use an <img> tag so i can set a style that upscales 32x32 to 64x64 and sets all the browser flags for nearest neighbor scaling
            let mut builder = String::new();
            for url in urls {
                let mut state_name = url.0;
                // Mark default states
                if state_name.is_empty() {
                    state_name = "{{DEFAULT}}".to_string();
                }

                builder.push_str(&format!(
                    include_str!(concat!(
                        env!("CARGO_MANIFEST_DIR"),
                        "/templates/diff_line.txt"
                    )),
                    state_name = state_name,
                    old = "",
                    new = url.1,
                    change_text = "Created",
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
            // dbg!(&before.icon.metadata);
            let urls = full_render(job, &before)
                .await
                .context("Failed to render deleted icon file")?;
            // dbg!(&urls);
            // TODO: tempted to use an <img> tag so i can set a style that upscales 32x32 to 64x64 and sets all the browser flags for nearest neighbor scaling
            let mut builder = String::new();
            for url in urls {
                let mut state_name = url.0;
                // Mark default states
                if state_name.is_empty() {
                    state_name = "{{DEFAULT}}".to_string();
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
                    change_text = "Deleted",
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
            let before_states: HashSet<String> =
                before.icon.metadata.state_names.keys().cloned().collect();
            let after_states: HashSet<String> =
                after.icon.metadata.state_names.keys().cloned().collect();

            let access = job.lock().await;
            let prefix = format!("{}/{}", access.installation, access.pull_request);
            drop(access);

            let mut builder = String::new();
            let mut before_renderer = IconRenderer::new(&before.icon);
            let mut after_renderer = IconRenderer::new(&after.icon);

            for state in before_states.symmetric_difference(&after_states) {
                if before_states.contains(state) {
                    let (name, url) = render_state(
                        &prefix,
                        &before,
                        before.icon.metadata.get_icon_state(state).unwrap(),
                        &mut before_renderer,
                    )
                    .await
                    .with_context(|| format!("Failed to render before-state {state}"))?;
                    builder.push_str(&format!(
                        include_str!(concat!(
                            env!("CARGO_MANIFEST_DIR"),
                            "/templates/diff_line.txt"
                        )),
                        state_name = name,
                        old = url,
                        new = "",
                        change_text = "Deleted",
                    ));
                    builder.push('\n');
                } else {
                    let (name, url) = render_state(
                        &prefix,
                        &after,
                        after.icon.metadata.get_icon_state(state).unwrap(),
                        &mut after_renderer,
                    )
                    .await
                    .with_context(|| format!("Failed to render after-state {state}"))?;
                    builder.push_str(&format!(
                        include_str!(concat!(
                            env!("CARGO_MANIFEST_DIR"),
                            "/templates/diff_line.txt"
                        )),
                        state_name = name,
                        old = "",
                        new = url,
                        change_text = "Created",
                    ));
                    builder.push('\n');
                }
            }

            for state in before_states.intersection(&after_states) {
                let before_state_render = before_renderer.render_to_images(state)?;
                let after_state_render = after_renderer.render_to_images(state)?;

                if before_state_render != after_state_render {
                    let before_state = before.icon.metadata.get_icon_state(state).unwrap();
                    let after_state = after.icon.metadata.get_icon_state(state).unwrap();

                    let (_, before_url) =
                        render_state(&prefix, &before, before_state, &mut before_renderer)
                            .await
                            .with_context(|| {
                                format!("Failed to render modified before-state {state}")
                            })?;
                    let (_, after_url) =
                        render_state(&prefix, &after, after_state, &mut after_renderer)
                            .await
                            .with_context(|| {
                                format!("Failed to render modified before-state {state}")
                            })?;

                    builder.push_str(&format!(
                        include_str!(concat!(
                            env!("CARGO_MANIFEST_DIR"),
                            "/templates/diff_line.txt"
                        )),
                        state_name = state,
                        old = before_url,
                        new = after_url,
                        change_text = "Modified",
                    ));
                    builder.push('\n');
                } else {
                    println!("No difference detected for {}", state);
                }
            }

            Ok(format!(
                include_str!(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/templates/diff_modify.txt"
                )),
                filename = before.name,
                table = builder
            ))
        }
    }
}

async fn render_state<'a, S: AsRef<str>>(
    prefix: S,
    target: &IconFileWithName,
    state: &State,
    renderer: &mut IconRenderer<'a>,
) -> Result<(String, String)> {
    let directory = Path::new(".").join("images").join(prefix.as_ref());
    // Always remember to mkdir -p your paths
    std::fs::create_dir_all(&directory)
        .with_context(|| format!("Failed to create directory {:?}", directory))?;

    let filename = format!(
        "{}-{}-{}-{}",
        // Differentiate between before-after files
        &target.sha,
        // Differentiate between different files in the same commit
        &target.name.replace(".dmi", ""),
        // Differentiate between duplicate states
        state.duplicate.unwrap_or(0),
        // Diffentiate between states.
        sanitize_filename::sanitize(&state.name)
    );

    let path = directory.join(&filename);
    // dbg!(&path, &state.frames);
    let corrected_path = renderer
        .render_state(state, path)
        .with_context(|| format!("Failed to render state {}", state.name))?;
    let extension = corrected_path
        .extension()
        .ok_or_else(|| format_err!("Unable to get extension that was written to"))?;
    // dbg!(&corrected_path, &extension);

    let url = format!(
        "{}/{}/{}.{}",
        CONFIG.get().unwrap().file_hosting_url,
        prefix.as_ref(),
        filename,
        extension.to_string_lossy()
    );

    Ok((state.get_state_name_index(), url))
}

async fn full_render(
    job: Arc<Mutex<&Job>>,
    target: &IconFileWithName,
) -> Result<Vec<(String, String)>> {
    let icon = &target.icon;

    let mut vec = Vec::new();

    let mut renderer = IconRenderer::new(icon);

    let access = job.lock().await;
    let prefix = format!("{}/{}", access.installation, access.pull_request);
    drop(access);

    for state in icon.metadata.states.iter() {
        let (name, url) = render_state(&prefix, target, state, &mut renderer)
            .await
            .with_context(|| format!("Failed to render state {}", state.name))?;
        vec.push((name, url));
    }

    // dbg!(&vec);

    Ok(vec)
}
