use eyre::{Context, Result};
use path_absolutize::Absolutize;
use secrecy::ExposeSecret;
use std::io::Read;
use std::path::Path;
use std::path::PathBuf;

use super::git_operations::{
    clean_up_references, clone_repo, fetch_and_get_branches, with_checkout,
};

use crate::rendering::{
    get_map_diff_bounding_boxes, load_maps, load_maps_with_whole_map_regions, render_diffs,
    render_map_regions, MapWithRegions, MapsWithRegions, RenderingContext,
};

use crate::CONFIG;

use diffbot_lib::{
    github::github_types::{
        Branch, ChangeType, CheckOutputBuilder, CheckOutputs, FileDiff, Output,
    },
    job::types::Job,
    tracing,
};

use super::Azure;

use rayon::prelude::*;
use serde::Deserialize;

struct RenderedMaps {
    added_maps: Vec<(String, MapWithRegions)>,
    removed_maps: Vec<(String, MapWithRegions)>,
    modified_maps: MapsWithRegions,
}

#[derive(Deserialize)]
struct MapConfig {
    include_pass: String,
    exclude_pass: String,
}

fn render(
    base: &Branch,
    head: &Branch,
    (added_files, modified_files, removed_files): (&[&FileDiff], &[&FileDiff], &[&FileDiff]),
    (repo, base_branch_name): (&git2::Repository, &str),
    (repo_dir, out_dir, blob_client): (&Path, &Path, Azure),
    pull_request_number: u64,
    // feel like this is a bit of a hack but it works for now
) -> Result<RenderedMaps> {
    tracing::debug!(
        "Fetching and getting branches, base: {:?}, head: {:?}",
        base,
        head
    );

    let pull_branch = format!("mdb-{}-{}", base.sha, head.sha);
    let head_branch = format!("pull/{pull_request_number}/head:{pull_branch}");

    let (base_branch, head_branch) =
        fetch_and_get_branches(&base.sha, &head.sha, repo, &head_branch, base_branch_name)
            .wrap_err("Fetching and constructing diffs")?;

    let path = repo_dir
        .absolutize()
        .wrap_err("Making repo path absolute")?;

    let base_context = with_checkout(&base_branch, repo, || RenderingContext::new(&path))
        .wrap_err("Parsing base")?;

    let head_context = with_checkout(&head_branch, repo, || RenderingContext::new(&path))
        .wrap_err("Parsing head")?;

    let config = with_checkout(&head_branch, repo, || {
        let config_path = path.to_path_buf().join("mapdiff.toml");
        let mut config_str = String::new();
        std::fs::File::open(config_path)?.read_to_string(&mut config_str)?;
        let config: MapConfig = toml::from_str(&config_str)?;
        Ok(config)
    })
    .unwrap_or_else(|_| MapConfig {
        include_pass: "".to_owned(),
        exclude_pass: "hide-space,hide-invisible,random".to_owned(),
    });

    let base_render_passes = dmm_tools::render_passes::configure(
        base_context.map_config(),
        &config.include_pass,
        &config.exclude_pass,
    );

    let head_render_passes = dmm_tools::render_passes::configure(
        head_context.map_config(),
        &config.include_pass,
        &config.exclude_pass,
    );

    //do removed maps
    let removed_directory = out_dir.to_path_buf().join("r");
    let removed_directory = removed_directory.as_path();

    let removed_errors = Default::default();

    let removed_maps = with_checkout(&base_branch, repo, || {
        let maps = load_maps_with_whole_map_regions(removed_files, &path)
            .wrap_err("Loading removed maps")?;
        render_map_regions(
            &base_context,
            maps.par_iter().map(|(k, v)| (k.as_str(), v)),
            &base_render_passes,
            (removed_directory, blob_client.clone()),
            "removed.png",
            &removed_errors,
            crate::rendering::MapType::Base,
        )
        .wrap_err("Rendering removed maps")?;
        Ok(maps)
    })?;

    //do added maps
    let added_directory = out_dir.to_path_buf().join("a");
    let added_directory = added_directory.as_path();

    let added_errors = Default::default();

    let added_maps = with_checkout(&head_branch, repo, || {
        let maps =
            load_maps_with_whole_map_regions(added_files, &path).wrap_err("Loading added maps")?;
        render_map_regions(
            &head_context,
            maps.par_iter().map(|(k, v)| (k.as_str(), v)),
            &head_render_passes,
            (added_directory, blob_client.clone()),
            "added.png",
            &added_errors,
            crate::rendering::MapType::Head,
        )
        .wrap_err("Rendering added maps")?;
        Ok(maps)
    })?;

    //do modified maps
    let base_maps = with_checkout(&base_branch, repo, || Ok(load_maps(modified_files, &path)))
        .wrap_err("Loading base maps")?;
    let mut head_maps = with_checkout(&head_branch, repo, || Ok(load_maps(modified_files, &path)))
        .wrap_err("Loading head maps")?;

    let modified_maps = base_maps
        .into_iter()
        .map(|(k, v)| {
            (
                k.clone(),
                (
                    v,
                    head_maps.shift_remove(&k).expect(
                        "head maps has maps that isn't inside base maps on modified comparison",
                    ),
                ),
            )
        })
        .collect::<indexmap::IndexMap<_, _, ahash::RandomState>>();

    if !head_maps.is_empty() {
        return Err(eyre::eyre!(
            "Did not account for the following maps in head_maps (this shouldn't happen): {:?}",
            head_maps.keys().collect::<Vec<_>>()
        ));
    }

    let modified_maps = get_map_diff_bounding_boxes(modified_maps)?;

    let modified_directory = out_dir.to_path_buf().join("m");
    let modified_directory = modified_directory.as_path();

    let modified_before_errors = Default::default();
    let modified_after_errors = Default::default();

    let before = with_checkout(&base_branch, repo, || {
        render_map_regions(
            &base_context,
            modified_maps
                .par_iter()
                .filter_map(|(map_name, (before, _))| {
                    Some((map_name.as_str(), before.as_ref().ok()?))
                }),
            &head_render_passes,
            (modified_directory, blob_client.clone()),
            "before.png",
            &modified_before_errors,
            crate::rendering::MapType::Base,
        )
        .wrap_err("Rendering modified before maps")
    })?;

    let after = with_checkout(&head_branch, repo, || {
        render_map_regions(
            &head_context,
            modified_maps
                .par_iter()
                .filter_map(|(map_name, (_, after))| Some((map_name.as_str(), after.as_ref()?))),
            &head_render_passes,
            (modified_directory, blob_client.clone()),
            "after.png",
            &modified_after_errors,
            crate::rendering::MapType::Head,
        )
        .wrap_err("Rendering modified after maps")
    })?;

    render_diffs(before, after, blob_client.clone());

    Ok(RenderedMaps {
        added_maps,
        modified_maps,
        removed_maps,
    })
}

fn generate_finished_output<P: AsRef<Path>>(
    file_directory: &P,
    maps: RenderedMaps,
) -> Result<CheckOutputs> {
    let conf = CONFIG.get().unwrap();
    let file_url = if conf.azure_blobs.is_some() {
        format!(
            "https://{}.blob.core.windows.net/{}",
            conf.azure_blobs.as_ref().unwrap().storage_account,
            conf.azure_blobs.as_ref().unwrap().storage_container
        )
    } else {
        conf.web.file_hosting_url.to_string()
    };
    let non_abs_directory = file_directory
        .as_ref()
        .to_string_lossy()
        .to_string()
        .replace('\\', "/");

    let mut builder = CheckOutputBuilder::new("Map renderings", &crate::read_config().summary_msg);

    let link_base = format!("{file_url}/{non_abs_directory}");

    // Those are CPU bound but parallelizing would require builder to be thread safe and it's probably not worth the overhead
    maps.added_maps.iter().for_each(|(file, map)| {
        let file_index = file.clone().replace('/', "_").replace(".dmm", "");
        map.iter_levels().for_each(|(level, _)| {
            let link = format!("{link_base}/a/{file_index}/{level}-added.png");
            let name = format!("{file} (Z-level: {})", level + 1);

            builder.add_text(&format!(
                include_str!("../templates/diff_template_add.txt"),
                filename = name,
                image_link = link
            ));
        });
    });

    maps.removed_maps.iter().for_each(|(file, map)| {
        let file_index = file.clone().replace('/', "_").replace(".dmm", "");
        map.iter_levels().for_each(|(level, _)| {
            let link = format!("{link_base}/r/{file_index}/{level}-removed.png");
            let name = format!("{file} (Z-level: {})", level + 1);

            builder.add_text(&format!(
                include_str!("../templates/diff_template_remove.txt"),
                filename = name,
                image_link = link
            ));
        });
    });

    const Z_DELETED_TEXT: &str = "Z-LEVEL DELETED";
    const Z_ADDED_TEXT: &str = "Z-LEVEL ADDED";
    const ROW_DESC: &str = "If the image doesn't load, use the raw link above";

    maps.modified_maps
        .iter()
        .for_each(|(file, (before, _))| match before {
            Ok(map) => {
                let file_index = file.clone().replace('/', "_").replace(".dmm", "");
                map.iter_levels().for_each(|(level, region)| {
                    let link = format!("{link_base}/m/{file_index}/{level}");
                    let name = format!("{file} (Z-level: {})", level + 1);
                    let (dim_x, dim_y, _) = map.map.dim_xyz();
                    let fmt_dim = format!("({dim_x}, {dim_y}, {})", level + 1);

                    let (link_before, link_after, link_diff) = (
                        format!("{link}-before.png"),
                        format!("{link}-after.png"),
                        format!("{link}-diff.png"),
                    );

                    match region {
                        crate::rendering::BoundType::None => (),
                        crate::rendering::BoundType::OnlyHead => {
                            builder.add_text(&format!(
                                include_str!("../templates/diff_template_mod.txt"),
                                bounds = fmt_dim,
                                filename = name,
                                image_before_link = "Unavailable",
                                image_after_link = format_args!("[New]({link_after})"),
                                image_diff_link = "Unavailable",
                                old_row = Z_ADDED_TEXT,
                                new_row = format_args!("![{ROW_DESC}]({link_after})"),
                                diff_row = Z_ADDED_TEXT
                            ));
                        }
                        crate::rendering::BoundType::OnlyBase => {
                            builder.add_text(&format!(
                                include_str!("../templates/diff_template_mod.txt"),
                                bounds = fmt_dim,
                                filename = name,
                                image_before_link = "Unavailable",
                                image_after_link = "Unavailable",
                                image_diff_link = "Unavailable",
                                old_row = Z_DELETED_TEXT,
                                new_row = Z_DELETED_TEXT,
                                diff_row = Z_DELETED_TEXT
                            ));
                        }
                        crate::rendering::BoundType::Both(bounds) => {
                            builder.add_text(&format!(
                                include_str!("../templates/diff_template_mod.txt"),
                                bounds = bounds.to_string(),
                                filename = name,
                                image_before_link = format_args!("[Old]({link_before})"),
                                image_after_link = format_args!("[New]({link_after})"),
                                image_diff_link = format_args!("[Diff]({link_diff})"),
                                old_row = format_args!("![{ROW_DESC}]({link_before})"),
                                new_row = format_args!("![{ROW_DESC}]({link_after})"),
                                diff_row = format_args!("![{ROW_DESC}]({link_diff})")
                            ));
                        }
                    }
                });
            }
            Err(e) => {
                let error = format!("{e:?}");
                builder.add_text(&format!(
                    include_str!("../templates/diff_template_error.txt"),
                    filename = file,
                    error = error,
                ));
            }
        });

    Ok(builder.build())
}

pub fn do_job(job: Job, blob_client: Azure) -> Result<CheckOutputs> {
    tracing::debug!(
        "Starting Job on repo: {}, pr number: {}, base commit: {}, head commit: {}",
        job.repo.full_name(),
        job.pull_request,
        job.base.sha,
        job.head.sha
    );

    let base = &job.base;
    let head = &job.head;

    let handle = actix_web::rt::Runtime::new()?;
    let (_, secret_token) =
        handle.block_on(octocrab::instance().installation_and_token(job.installation))?;

    let repo_dir: PathBuf = ["./repos/", &job.repo.full_name()].iter().collect();

    let url = format!(
        "https://x-access-token:{}@github.com/{}",
        secret_token.expose_secret(),
        job.repo.full_name()
    );
    let clone_required = !repo_dir.exists();
    if clone_required {
        tracing::debug!("Directory {:?} doesn't exist, creating dir", repo_dir);
        std::fs::create_dir_all(&repo_dir)?;
        handle.block_on(async {
                let output = Output {
                    title: "Cloning repo...",
                    summary: "The repository is being cloned, this will take a few minutes. Future runs will not require cloning.".to_owned(),
                    text: "".to_owned(),
                };
                _ = job.check_run.set_output(output).await; // we don't really care if updating the job fails, just continue
            });
        clone_repo(&url, &repo_dir).wrap_err("Cloning repo")?;
    }

    let non_abs_directory: PathBuf = [
        "images".to_string(),
        job.repo.id.to_string(),
        job.check_run.id().to_string(),
    ]
    .iter()
    .collect();
    let output_directory = non_abs_directory
        .as_path()
        .absolutize()
        .wrap_err("Absolutizing images path")?;

    tracing::debug!(
        "Dirs absolutized from {:?} to {:?}",
        non_abs_directory,
        output_directory
    );

    let filter_on_status = |status: ChangeType| {
        job.files
            .iter()
            .filter(|f| f.status == status)
            .collect::<Vec<&FileDiff>>()
    };

    let added_files = filter_on_status(ChangeType::Added);
    let modified_files = filter_on_status(ChangeType::Modified);
    let removed_files = filter_on_status(ChangeType::Deleted);

    let repository = git2::Repository::open(&repo_dir).wrap_err("Opening repository")?;

    if !clone_required {
        repository.remote_set_url("origin", &url)?;
    }

    let mut remote = repository.find_remote("origin")?;

    remote
        .connect(git2::Direction::Fetch)
        .wrap_err("Connecting to remote")?;

    remote.disconnect().wrap_err("Disconnecting from remote")?;

    let output_directory = if blob_client.is_some() {
        Path::new(&non_abs_directory)
    } else {
        output_directory.as_ref()
    };

    let res = match render(
        base,
        head,
        (&added_files, &modified_files, &removed_files),
        (&repository, &job.base.r#ref),
        (&repo_dir, output_directory, blob_client),
        job.pull_request,
    )
    .wrap_err("")
    {
        Ok(maps) => generate_finished_output(&non_abs_directory, maps),
        Err(err) => Err(err),
    };

    clean_up_references(&repository, &job.base.r#ref).wrap_err("Cleaning up references")?;

    res
}
