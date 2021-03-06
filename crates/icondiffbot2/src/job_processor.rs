use crate::{
    sha::{sha_to_iconfile, status_to_sha, IconFileWithName},
    table_builder::OutputTableBuilder,
    CONFIG,
};
use anyhow::{Context, Result};
use diffbot_lib::{github::github_types::CheckOutputs, job::types::Job};
use dmm_tools::dmi::render::{IconRenderer, RenderType};
use dmm_tools::dmi::State;
use dreammaker::dmi::StateIndex;
use hashbrown::HashSet;
use rayon::prelude::*;
use std::{
    collections::hash_map::DefaultHasher,
    fs::File,
    hash::{Hash, Hasher},
    io::{BufWriter, Write},
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
        let file = sha_to_iconfile(job, &dmi.filename, status_to_sha(job, &dmi.status)).await?;

        let j = job.clone();
        let states = tokio::task::spawn_blocking(move || render(&j, file)).await??;

        map.insert(dmi.filename.as_str(), states);
    }

    map.build().instrument(info_span!("Building table")).await
}

#[tracing::instrument]
fn render(
    job: &Job,
    diff: (Option<IconFileWithName>, Option<IconFileWithName>),
) -> Result<(String, Vec<String>)> {
    // TODO: Alphabetize
    // TODO: Test more edge cases
    match diff {
        (None, None) => unreachable!("Diffing (None, None) makes no sense"),
        (None, Some(after)) => {
            let urls = full_render(job, &after).context("Failed to render new icon file")?;

            Ok((
                "ADDED".to_owned(),
                urls.par_iter()
                    .map(|(state_name, url)| {
                        format!(
                            include_str!(concat!(
                                env!("CARGO_MANIFEST_DIR"),
                                "/templates/diff_line.txt"
                            )),
                            state_name = state_name,
                            old = "",
                            new = url,
                            change_text = "Created",
                        )
                    })
                    .collect(),
            ))
        }
        (Some(before), None) => {
            let urls = full_render(job, &before).context("Failed to render deleted icon file")?;

            Ok((
                "DELETED".to_owned(),
                urls.par_iter()
                    .map(|(state_name, url)| {
                        format!(
                            include_str!(concat!(
                                env!("CARGO_MANIFEST_DIR"),
                                "/templates/diff_line.txt"
                            )),
                            state_name = state_name,
                            old = url,
                            new = "",
                            change_text = "Deleted",
                        )
                    })
                    .collect(),
            ))
        }
        (Some(before), Some(after)) => {
            let before_states: HashSet<&StateIndex> =
                before.icon.metadata.state_names.keys().collect();
            let after_states: HashSet<&StateIndex> =
                after.icon.metadata.state_names.keys().collect();

            let prefix = format!("{}/{}", job.installation, job.pull_request);

            let before_renderer = IconRenderer::new(&before.icon);
            let after_renderer = IconRenderer::new(&after.icon);

            let mut table: Vec<String> = before_states
                .par_symmetric_difference(&after_states)
                .map(|state| {
                    if before_states.contains(state) {
                        let (name, url) = render_state(
                            &prefix,
                            &before,
                            before.icon.metadata.get_icon_state(state).unwrap(),
                            &before_renderer,
                        )
                        .with_context(|| format!("Failed to render before-state {state}"))?;
                        Ok(format!(
                            include_str!(concat!(
                                env!("CARGO_MANIFEST_DIR"),
                                "/templates/diff_line.txt"
                            )),
                            state_name = name,
                            old = url,
                            new = "",
                            change_text = "Deleted",
                        ))
                    } else {
                        let (name, url) = render_state(
                            &prefix,
                            &after,
                            after.icon.metadata.get_icon_state(state).unwrap(),
                            &after_renderer,
                        )
                        .with_context(|| format!("Failed to render after-state {state}"))?;
                        Ok(format!(
                            include_str!(concat!(
                                env!("CARGO_MANIFEST_DIR"),
                                "/templates/diff_line.txt"
                            )),
                            state_name = name,
                            old = "",
                            new = url,
                            change_text = "Created",
                        ))
                    }
                })
                .filter_map(|r: Result<String, anyhow::Error>| {
                    r.map_err(|e| {
                        println!("Error encountered during parse: {}", e);
                    })
                    .ok()
                })
                .collect();

            table.par_extend(
                before_states
                    .par_intersection(&after_states)
                    .map(|state| {
                        let before_state = before.icon.metadata.get_icon_state(state).unwrap();
                        let after_state = after.icon.metadata.get_icon_state(state).unwrap();

                        let difference = {
                            // #[cfg(debug_assertions)]
                            // dbg!(before_state, after_state);
                            if before_state != after_state {
                                true
                            } else {
                                let before_state_render =
                                    before_renderer.render_to_images(state)?;
                                let after_state_render = after_renderer.render_to_images(state)?;
                                before_state_render != after_state_render
                            }
                        };

                        if difference {
                            let before_state = before.icon.metadata.get_icon_state(state).unwrap();
                            let after_state = after.icon.metadata.get_icon_state(state).unwrap();

                            let (_, before_url) =
                                render_state(&prefix, &before, before_state, &before_renderer)
                                    .with_context(|| {
                                        format!("Failed to render modified before-state {state}")
                                    })?;
                            let (_, after_url) =
                                render_state(&prefix, &after, after_state, &after_renderer)
                                    .with_context(|| {
                                        format!("Failed to render modified before-state {state}")
                                    })?;

                            Ok(format!(
                                include_str!(concat!(
                                    env!("CARGO_MANIFEST_DIR"),
                                    "/templates/diff_line.txt"
                                )),
                                state_name = state,
                                old = before_url,
                                new = after_url,
                                change_text = "Modified",
                            ))
                        } else {
                            Ok("".to_string())
                        }
                    })
                    .filter_map(|r: Result<String, anyhow::Error>| {
                        r.map_err(|e| {
                            println!("Error encountered during parse: {}", e);
                        })
                        .ok()
                    })
                    .filter(|s| !s.is_empty()),
            );

            Ok(("MODIFIED".to_owned(), table))
        }
    }
}

#[tracing::instrument]
fn render_state<'a, S: AsRef<str> + std::fmt::Debug>(
    prefix: S,
    target: &IconFileWithName,
    state: &State,
    renderer: &IconRenderer<'a>,
) -> Result<(StateIndex, String)> {
    let directory = Path::new(".").join("images").join(prefix.as_ref());
    // Always remember to mkdir -p your paths
    std::fs::create_dir_all(&directory)
        .with_context(|| format!("Failed to create directory {:?}", directory))?;

    let mut hasher = DefaultHasher::new();
    target.sha.hash(&mut hasher);
    target.full_name.hash(&mut hasher);
    target.hash.hash(&mut hasher);
    state.duplicate_index.hash(&mut hasher);
    state.name.hash(&mut hasher);
    let filename = hasher.finish().to_string();

    // TODO: Calculate file extension separately so that we can Error here if we overwrite a file
    let mut path = directory.join(&filename);

    let render_guard = renderer
        .prepare_render_state(state)
        .with_context(|| format!("Failed to create render guard for state {}", state.name))?;

    let extension = match render_guard.render_type {
        RenderType::Png => "png",
        RenderType::Gif => "gif",
    };
    path.set_extension(extension);

    let mut buffer = BufWriter::new(File::create(&path)?);

    render_guard
        .render(&mut buffer)
        .with_context(|| format!("Failed to render state {} to file {:?}", state.name, &path))?;

    let url = format!(
        "{}/{}/{}.{}",
        CONFIG.get().unwrap().web.file_hosting_url,
        prefix.as_ref(),
        filename,
        extension,
    );

    buffer.flush().with_context(|| {
        format!(
            "Failed to flush BufWriter to disk for state {:?} at {:?}",
            state, &path
        )
    })?;

    Ok((state.get_state_name_index(), url))
}

#[tracing::instrument]
fn full_render(job: &Job, target: &IconFileWithName) -> Result<Vec<(StateIndex, String)>> {
    let icon = &target.icon;

    let renderer = IconRenderer::new(icon);

    let prefix = format!("{}/{}", job.installation, job.pull_request);

    let vec: Vec<(StateIndex, String)> = icon
        .metadata
        .states
        .par_iter()
        .map(|state| {
            render_state(&prefix, target, state, &renderer)
                .with_context(|| format!("Failed to render state {}", state.name))
        })
        .filter_map(|r: Result<(StateIndex, String), anyhow::Error>| {
            r.map_err(|e| {
                println!("Error encountered during parse: {}", e);
            })
            .ok()
        })
        .collect();

    Ok(vec)
}
