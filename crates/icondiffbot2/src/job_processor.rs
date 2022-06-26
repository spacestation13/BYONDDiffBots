use crate::{
    sha::{sha_to_iconfile, status_to_sha, IconFileWithName},
    table_builder::OutputTableBuilder,
    CONFIG,
};
use anyhow::{format_err, Context, Result};
use diffbot_lib::{github::github_types::CheckOutputs, job::types::Job};
use dmm_tools::dmi::render::IconRenderer;
use dmm_tools::dmi::State;
use std::{
    collections::{hash_map::DefaultHasher, HashSet},
    hash::{Hash, Hasher},
    path::Path,
};
use tokio::runtime::Handle;
use tracing::{info_span, Instrument};

#[tracing::instrument]
pub fn do_job(job: &Job) -> Result<CheckOutputs> {
    // TODO: Maybe have jobs just be async?
    let handle = Handle::try_current()?;
    handle.block_on(async { handle_changed_files(job).await })
}

#[tracing::instrument]
pub async fn handle_changed_files(job: &Job) -> Result<CheckOutputs> {
    job.check_run.mark_started().await?;

    let mut map = OutputTableBuilder::new();

    for dmi in &job.files {
        let states = render(
            job,
            sha_to_iconfile(job, &dmi.filename, status_to_sha(job, &dmi.status)).await?,
        )
        .await?;
        map.insert(dmi.filename.as_str(), states);
    }

    map.build().instrument(info_span!("Building table")).await
}

#[tracing::instrument]
async fn render(
    job: &Job,
    diff: (Option<IconFileWithName>, Option<IconFileWithName>),
) -> Result<(String, Vec<String>)> {
    // TODO: Alphabetize
    // TODO: Test more edge cases
    // TODO: Parallelize?
    match diff {
        (None, None) => unreachable!("Diffing (None, None) makes no sense"),
        (None, Some(after)) => {
            let urls = full_render(job, &after)
                .await
                .context("Failed to render new icon file")?;
            let mut builder = Vec::new();
            for url in urls {
                let mut state_name = url.0;
                // Mark default states
                if state_name.is_empty() {
                    state_name = "{{DEFAULT}}".to_string();
                }

                builder.push(format!(
                    include_str!(concat!(
                        env!("CARGO_MANIFEST_DIR"),
                        "/templates/diff_line.txt"
                    )),
                    state_name = state_name,
                    old = "",
                    new = url.1,
                    change_text = "Created",
                ));
            }

            Ok(("ADDED".to_owned(), builder))
        }
        (Some(before), None) => {
            // dbg!(&before.icon.metadata);
            let urls = full_render(job, &before)
                .await
                .context("Failed to render deleted icon file")?;
            // dbg!(&urls);
            let mut builder = Vec::new();
            for url in urls {
                let mut state_name = url.0;
                // Mark default states
                if state_name.is_empty() {
                    state_name = "{{DEFAULT}}".to_string();
                }

                // Build the output line
                builder.push(format!(
                    include_str!(concat!(
                        env!("CARGO_MANIFEST_DIR"),
                        "/templates/diff_line.txt"
                    )),
                    state_name = state_name,
                    old = url.1,
                    new = "",
                    change_text = "Deleted",
                ));
            }

            Ok(("DELETED".to_owned(), builder))
        }
        (Some(before), Some(after)) => {
            let before_states: HashSet<&String> = before.icon.metadata.state_names.keys().collect();
            let after_states: HashSet<&String> = after.icon.metadata.state_names.keys().collect();

            let prefix = format!("{}/{}", job.installation, job.pull_request);

            let mut builder = Vec::new();
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
                    builder.push(format!(
                        include_str!(concat!(
                            env!("CARGO_MANIFEST_DIR"),
                            "/templates/diff_line.txt"
                        )),
                        state_name = name,
                        old = url,
                        new = "",
                        change_text = "Deleted",
                    ));
                } else {
                    let (name, url) = render_state(
                        &prefix,
                        &after,
                        after.icon.metadata.get_icon_state(state).unwrap(),
                        &mut after_renderer,
                    )
                    .await
                    .with_context(|| format!("Failed to render after-state {state}"))?;
                    builder.push(format!(
                        include_str!(concat!(
                            env!("CARGO_MANIFEST_DIR"),
                            "/templates/diff_line.txt"
                        )),
                        state_name = name,
                        old = "",
                        new = url,
                        change_text = "Created",
                    ));
                }
            }

            for state in before_states.intersection(&after_states) {
                let before_state = before.icon.metadata.get_icon_state(state).unwrap();
                let after_state = after.icon.metadata.get_icon_state(state).unwrap();

                let difference = {
                    // #[cfg(debug_assertions)]
                    // dbg!(before_state, after_state);
                    if before_state != after_state {
                        true
                    } else {
                        let before_state_render = before_renderer.render_to_images(state)?;
                        let after_state_render = after_renderer.render_to_images(state)?;
                        before_state_render != after_state_render
                    }
                };

                if difference {
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

                    builder.push(format!(
                        include_str!(concat!(
                            env!("CARGO_MANIFEST_DIR"),
                            "/templates/diff_line.txt"
                        )),
                        state_name = state,
                        old = before_url,
                        new = after_url,
                        change_text = "Modified",
                    ));
                }
                /* else {
                    println!("No difference detected for {}", state);
                } */
            }

            Ok(("MODIFIED".to_owned(), builder))
        }
    }
}

#[tracing::instrument]
async fn render_state<'a, S: AsRef<str> + std::fmt::Debug>(
    prefix: S,
    target: &IconFileWithName,
    state: &State,
    renderer: &mut IconRenderer<'a>,
) -> Result<(String, String)> {
    let directory = Path::new(".").join("images").join(prefix.as_ref());
    // Always remember to mkdir -p your paths
    std::fs::create_dir_all(&directory)
        .with_context(|| format!("Failed to create directory {:?}", directory))?;

    let mut hasher = DefaultHasher::new();
    target.sha.hash(&mut hasher);
    target.full_name.hash(&mut hasher);
    target.hash.hash(&mut hasher);
    state.duplicate.unwrap_or(0).hash(&mut hasher);
    state.name.hash(&mut hasher);
    let filename = hasher.finish().to_string();

    // TODO: Calculate file extension separately so that we can Error here if we overwrite a file
    let path = directory.join(&filename);
    // dbg!(&path, &state.frames);
    let corrected_path = renderer
        .render_state(state, &path)
        .with_context(|| format!("Failed to render state {} to file {:?}", state.name, path))?;
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

#[tracing::instrument]
async fn full_render(job: &Job, target: &IconFileWithName) -> Result<Vec<(String, String)>> {
    let icon = &target.icon;

    let mut vec = Vec::new();

    let mut renderer = IconRenderer::new(icon);

    let prefix = format!("{}/{}", job.installation, job.pull_request);

    for state in icon.metadata.states.iter() {
        let (name, url) = render_state(&prefix, target, state, &mut renderer)
            .await
            .with_context(|| format!("Failed to render state {}", state.name))?;
        vec.push((name, url));
    }

    // dbg!(&vec);

    Ok(vec)
}
