use crate::{
    sha::{sha_to_iconfile, status_to_sha, IconFileWithName},
    table_builder::OutputTableBuilder,
    CONFIG,
};
use diffbot_lib::log::error;
use diffbot_lib::{github::github_types::CheckOutputs, job::types::Job};
use dmm_tools::dmi::render::{IconRenderer, RenderType};
use dmm_tools::dmi::State;
use eyre::{Context, Result};
use hashbrown::HashSet;
use rayon::prelude::*;
use std::{
    fs::File,
    hash::{Hash, Hasher},
    io::{BufWriter, Write},
    path::Path,
};

#[tracing::instrument]
pub fn do_job(job: Job) -> Result<CheckOutputs> {
    let handle = actix_web::rt::Runtime::new()?;

    handle.block_on(async { job.check_run.mark_started().await })?;

    let mut map = OutputTableBuilder::new();

    for dmi in &job.files {
        let file = sha_to_iconfile(&job, &dmi.filename, status_to_sha(&job, &dmi.status))?;

        let states = render(&job, file)?;

        map.insert(dmi.filename.as_str(), states);
    }

    map.build()
}

#[tracing::instrument]
fn render(
    job: &Job,
    diff: (Option<IconFileWithName>, Option<IconFileWithName>),
) -> Result<(&'static str, Vec<String>)> {
    // TODO: Alphabetize
    // TODO: Test more edge cases
    match diff {
        (None, None) => Ok((
            "UNCHANGED",
            vec![format!(
                include_str!(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/templates/diff_line.txt"
                )),
                state_name = "",
                old = "",
                new = "",
                change_text = "UNCHANGED",
            )],
        )),
        (None, Some(after)) => {
            let urls = full_render(job, &after).context("Failed to render new icon file")?;

            Ok((
                "ADDED",
                urls.par_iter()
                    .map(|(state_name, url)| {
                        format!(
                            include_str!(concat!(
                                env!("CARGO_MANIFEST_DIR"),
                                "/templates/diff_line.txt"
                            )),
                            state_name = format!("{}:{}", state_name.0, state_name.1),
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
                "DELETED",
                urls.par_iter()
                    .map(|(state_name, url)| {
                        format!(
                            include_str!(concat!(
                                env!("CARGO_MANIFEST_DIR"),
                                "/templates/diff_line.txt"
                            )),
                            state_name = format!("{}:{}", state_name.0, state_name.1),
                            old = url,
                            new = "",
                            change_text = "Deleted",
                        )
                    })
                    .collect(),
            ))
        }
        (Some(before), Some(after)) => {
            let before_states: HashSet<(usize, &str), ahash::RandomState> = before
                .icon
                .metadata
                .states
                .values()
                .flatten()
                .map(|(idx, item)| (*idx, item.name.as_str()))
                .collect();
            let after_states: HashSet<(usize, &str), ahash::RandomState> = after
                .icon
                .metadata
                .states
                .values()
                .flatten()
                .map(|(idx, item)| (*idx, item.name.as_str()))
                .collect();

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
                            before.icon.metadata.get_icon_state(state.1).unwrap(),
                            &before_renderer,
                        )
                        .with_context(|| format!("Failed to render before-state {state:?}"))?;
                        Ok(format!(
                            include_str!(concat!(
                                env!("CARGO_MANIFEST_DIR"),
                                "/templates/diff_line.txt"
                            )),
                            state_name = format!("{}:{}", name.0, name.1),
                            old = url,
                            new = "",
                            change_text = "Deleted",
                        ))
                    } else {
                        let (name, url) = render_state(
                            &prefix,
                            &after,
                            after.icon.metadata.get_icon_state(state.1).unwrap(),
                            &after_renderer,
                        )
                        .with_context(|| format!("Failed to render after-state {state:?}"))?;
                        Ok(format!(
                            include_str!(concat!(
                                env!("CARGO_MANIFEST_DIR"),
                                "/templates/diff_line.txt"
                            )),
                            state_name = format!("{}:{}", name.0, name.1),
                            old = "",
                            new = url,
                            change_text = "Created",
                        ))
                    }
                })
                .filter_map(|r: Result<String, eyre::Error>| {
                    r.map_err(|e| {
                        error!("Error encountered during parse: {}", e);
                    })
                    .ok()
                })
                .collect();

            table.par_extend(
                before_states
                    .par_intersection(&after_states)
                    .map(|(_, state)| {
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
                    .filter_map(|r: Result<String, eyre::Error>| {
                        r.map_err(|e| {
                            error!("Error encountered during parse: {}", e);
                        })
                        .ok()
                    })
                    .filter(|s| !s.is_empty()),
            );

            Ok(("MODIFIED", table))
        }
    }
}

#[tracing::instrument]
fn render_state<'a, S: AsRef<str> + std::fmt::Debug>(
    prefix: S,
    target: &IconFileWithName,
    (index, state): (usize, &State),
    renderer: &IconRenderer<'a>,
) -> Result<((usize, String), String)> {
    let directory = Path::new(".").join("images").join(prefix.as_ref());
    // Always remember to mkdir -p your paths
    std::fs::create_dir_all(&directory)
        .with_context(|| format!("Failed to create directory {directory:?}"))?;

    let mut hasher = ahash::AHasher::default();
    target.sha.hash(&mut hasher);
    target.full_name.hash(&mut hasher);
    target.hash.hash(&mut hasher);
    index.hash(&mut hasher);
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
        format!("Failed to flush BufWriter to disk for state {state:?} at {path:?}")
    })?;

    Ok(((index, state.name.clone()), url))
}

#[tracing::instrument]
fn full_render(job: &Job, target: &IconFileWithName) -> Result<Vec<((usize, String), String)>> {
    let icon = &target.icon;

    let renderer = IconRenderer::new(icon);

    let prefix = format!("{}/{}", job.installation, job.pull_request);

    let vec: Vec<((usize, String), String)> = icon
        .metadata
        .states
        .par_values()
        .flatten()
        .map(|(idx, state)| {
            render_state(&prefix, target, (*idx, state), &renderer)
                .with_context(|| format!("Failed to render state {}", state.name))
        })
        .filter_map(|r: Result<((usize, String), String), eyre::Error>| {
            r.map_err(|e| {
                error!("Error encountered during parse: {}", e);
            })
            .ok()
        })
        .collect();

    Ok(vec)
}
