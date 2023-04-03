use diffbot_lib::log;
use eyre::{Context, Result};
use path_absolutize::Absolutize;
use rayon::prelude::*;
use std::path::Path;
use std::path::PathBuf;

use super::git_operations::{
    clean_up_references, clone_repo, fetch_and_get_branches, with_checkout,
};

use crate::rendering::{
    get_map_diff_bounding_boxes, load_maps, load_maps_with_whole_map_regions,
    render_diffs_for_directory, render_map_regions, MapWithRegions, MapsWithRegions,
    RenderingContext,
};

use crate::CONFIG;

use diffbot_lib::{
    github::github_types::{
        Branch, ChangeType, CheckOutputBuilder, CheckOutputs, FileDiff, Output,
    },
    job::types::Job,
};

struct RenderedMaps {
    added_maps: Vec<MapWithRegions>,
    removed_maps: Vec<MapWithRegions>,
    modified_maps: MapsWithRegions,
}

fn render(
    base: &Branch,
    head: &Branch,
    (added_files, modified_files, removed_files): (&[&FileDiff], &[&FileDiff], &[&FileDiff]),
    (repo, base_branch_name): (&git2::Repository, &str),
    (repo_dir, out_dir): (&Path, &Path),
    pull_request_number: u64,
    // feel like this is a bit of a hack but it works for now
) -> Result<RenderedMaps> {
    log::trace!(
        "Fetching and getting branches, base: {:?}, head: {:?}",
        base,
        head
    );

    let pull_branch = format!("mdb-{}-{}", base.sha, head.sha);
    let head_branch = format!("pull/{pull_request_number}/head:{pull_branch}");

    let (base_branch, head_branch) =
        fetch_and_get_branches(&base.sha, &head.sha, repo, &head_branch, base_branch_name)
            .context("Fetching and constructing diffs")?;

    let path = repo_dir.absolutize().context("Making repo path absolute")?;

    let base_context = with_checkout(&base_branch, repo, || RenderingContext::new(&path))
        .context("Parsing base")?;

    let head_context = with_checkout(&head_branch, repo, || RenderingContext::new(&path))
        .context("Parsing head")?;

    let base_render_passes = dmm_tools::render_passes::configure(
        base_context.map_config(),
        "",
        "hide-space,hide-invisible,random",
    );

    let head_render_passes = dmm_tools::render_passes::configure(
        head_context.map_config(),
        "",
        "hide-space,hide-invisible,random",
    );

    //do removed maps
    let removed_directory = format!("{}/r", out_dir.display());
    let removed_directory = Path::new(&removed_directory);
    let removed_errors = Default::default();

    let removed_maps = with_checkout(&base_branch, repo, || {
        let maps = load_maps_with_whole_map_regions(removed_files, &path)
            .context("Loading removed maps")?;
        render_map_regions(
            &base_context,
            &maps.iter().collect::<Vec<_>>(),
            &base_render_passes,
            removed_directory,
            "removed.png",
            &removed_errors,
        )
        .context("Rendering removed maps")?;
        Ok(maps)
    })?;

    //do added maps
    let added_directory = format!("{}/a", out_dir.display());
    let added_directory = Path::new(&added_directory);
    let added_errors = Default::default();

    let added_maps = with_checkout(&head_branch, repo, || {
        let maps =
            load_maps_with_whole_map_regions(added_files, &path).context("Loading added maps")?;
        render_map_regions(
            &head_context,
            &maps.iter().collect::<Vec<_>>(),
            &head_render_passes,
            added_directory,
            "added.png",
            &added_errors,
        )
        .context("Rendering added maps")?;
        Ok(maps)
    })
    .context("Rendering modified after and added maps")?;

    //do modified maps
    let base_maps = with_checkout(&base_branch, repo, || Ok(load_maps(modified_files, &path)))
        .context("Loading base maps")?;
    let head_maps = with_checkout(&head_branch, repo, || Ok(load_maps(modified_files, &path)))
        .context("Loading head maps")?;

    let modified_maps = get_map_diff_bounding_boxes(base_maps, head_maps)?;

    let modified_directory = format!("{}/m", out_dir.display());
    let modified_directory = Path::new(&modified_directory);
    let modified_before_errors = Default::default();
    let modified_after_errors = Default::default();

    with_checkout(&base_branch, repo, || {
        render_map_regions(
            &base_context,
            modified_maps
                .befores
                .iter()
                .filter_map(|res| res.as_ref().ok())
                .collect::<Vec<_>>()
                .as_slice(),
            &head_render_passes,
            modified_directory,
            "before.png",
            &modified_before_errors,
        )
        .context("Rendering modified before maps")?;
        Ok(())
    })?;

    with_checkout(&head_branch, repo, || {
        render_map_regions(
            &head_context,
            modified_maps
                .afters
                .iter()
                .filter_map(|opt| opt.as_ref())
                .collect::<Vec<_>>()
                .as_slice(),
            &head_render_passes,
            modified_directory,
            "after.png",
            &modified_after_errors,
        )
        .context("Rendering modified after maps")?;
        Ok(())
    })?;

    (0..modified_files.len()).into_par_iter().for_each(|i| {
        render_diffs_for_directory(modified_directory.join(i.to_string()));
    });

    Ok(RenderedMaps {
        added_maps,
        modified_maps,
        removed_maps,
    })
}

fn generate_finished_output<P: AsRef<Path>>(
    added_files: &[&FileDiff],
    modified_files: &[&FileDiff],
    removed_files: &[&FileDiff],
    file_directory: &P,
    maps: RenderedMaps,
) -> Result<CheckOutputs> {
    let conf = CONFIG.get().unwrap();
    let file_url = &conf.web.file_hosting_url;
    let non_abs_directory = file_directory.as_ref().to_string_lossy();

    let mut builder = CheckOutputBuilder::new(
    "Map renderings",
    "*Please file any issues [here](https://github.com/spacestation13/BYONDDiffBots/issues).*\n\nMaps with diff:",
    );

    let link_base = format!("{file_url}/{non_abs_directory}");

    // Those are CPU bound but parallelizing would require builder to be thread safe and it's probably not worth the overhead
    added_files
        .iter()
        .zip(maps.added_maps.iter())
        .enumerate()
        .for_each(|(file_index, (file, map))| {
            map.iter_levels().for_each(|(level, _)| {
                let link = format!("{link_base}/a/{file_index}/{level}-added.png");
                let name = format!("{}:{}", file.filename, level + 1);

                builder.add_text(&format!(
                    include_str!("../templates/diff_template_add.txt"),
                    filename = name,
                    image_link = link
                ));
            });
        });

    modified_files
        .iter()
        .zip(maps.modified_maps.befores.iter())
        .enumerate()
        .for_each(|(file_index, (file, map))| match map {
            Ok(map) => {
                map.iter_levels().for_each(|(level, region)| {
                    let link = format!("{link_base}/m/{file_index}/{level}");
                    let name = format!("{}:{}", file.filename, level + 1);

                    #[allow(clippy::format_in_format_args)]
                    builder.add_text(&format!(
                        include_str!("../templates/diff_template_mod.txt"),
                        bounds = region.to_string(),
                        filename = name,
                        image_before_link = format!("{link}-before.png"),
                        image_after_link = format!("{link}-after.png"),
                        image_diff_link = format!("{link}-diff.png")
                    ));
                });
            }
            Err(e) => {
                let error = format!("{e:?}");
                builder.add_text(&format!(
                    include_str!("../templates/diff_template_error.txt"),
                    filename = file.filename,
                    error = error,
                ));
            }
        });

    removed_files
        .iter()
        .zip(maps.removed_maps.iter())
        .enumerate()
        .for_each(|(file_index, (file, map))| {
            map.iter_levels().for_each(|(level, _)| {
                let link = format!("{link_base}/r/{file_index}/{level}-removed.png");
                let name = format!("{}:{}", file.filename, level + 1);

                builder.add_text(&format!(
                    include_str!("../templates/diff_template_remove.txt"),
                    filename = name,
                    image_link = link
                ));
            });
        });

    Ok(builder.build())
}

pub fn do_job(job: Job) -> Result<CheckOutputs> {
    log::trace!(
        "Starting Job on repo: {}, pr number: {}, base commit: {}, head commit: {}",
        job.repo.full_name(),
        job.pull_request,
        job.base.sha,
        job.head.sha
    );

    let base = &job.base;
    let head = &job.head;
    let repo = format!("https://github.com/{}", job.repo.full_name());
    let repo_dir: PathBuf = ["./repos/", &job.repo.full_name()].iter().collect();

    let handle = actix_web::rt::Runtime::new()?;

    if !repo_dir.exists() {
        log::trace!("Directory {:?} doesn't exist, creating dir", repo_dir);
        std::fs::create_dir_all(&repo_dir)?;
        handle.block_on(async {
                let output = Output {
                    title: "Cloning repo...",
                    summary: "The repository is being cloned, this will take a few minutes. Future runs will not require cloning.".to_owned(),
                    text: "".to_owned(),
                };
                let _ = job.check_run.set_output(output).await; // we don't really care if updating the job fails, just continue
            });
        clone_repo(&repo, &repo_dir).context("Cloning repo")?;
    }

    let non_abs_directory = format!("images/{}/{}", job.repo.id, job.check_run.id());
    let output_directory = Path::new(&non_abs_directory)
        .absolutize()
        .context("Absolutizing images path")?;
    let output_directory = output_directory
        .as_ref()
        .to_str()
        .ok_or_else(|| eyre::anyhow!("Failed to create absolute path to image directory",))?;

    log::trace!(
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

    let repository = git2::Repository::open(&repo_dir).context("Opening repository")?;

    let mut remote = repository.find_remote("origin")?;

    remote
        .connect(git2::Direction::Fetch)
        .context("Connecting to remote")?;

    remote.disconnect().context("Disconnecting from remote")?;

    let res = match render(
        base,
        head,
        (&added_files, &modified_files, &removed_files),
        (&repository, &job.base.r#ref),
        (&repo_dir, Path::new(output_directory)),
        job.pull_request,
    ) {
        Ok(maps) => generate_finished_output(
            &added_files,
            &modified_files,
            &removed_files,
            &non_abs_directory,
            maps,
        ),

        Err(err) => Err(err),
    };

    clean_up_references(&repository, &job.base.r#ref).context("Cleaning up references")?;

    res
}
