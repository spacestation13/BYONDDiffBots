use anyhow::{Context, Result};
use std::path::Path;

use git2::{build::CheckoutBuilder, Diff, FetchOptions, Repository};

pub fn with_repo_dir<T>(repo: &Path, f: impl FnOnce() -> Result<T>) -> Result<T> {
    std::env::set_current_dir(repo)?;
    let result = f();
    std::env::set_current_dir(std::env::current_exe()?.parent().unwrap())?;
    result
}

pub fn fetch_diffs_and_update<'a>(
    base_sha: &str,
    head_sha: &str,
    repo: &'a Repository,
    extra_branch: &str,
) -> Result<Diff<'a>> {
    let base_id = git2::Oid::from_str(base_sha).context("Parsing base sha")?;
    let head_id = git2::Oid::from_str(head_sha).context("Parsing head sha")?;

    let mut remote = repo.find_remote("origin")?;

    remote
        .connect(git2::Direction::Fetch)
        .context("Connecting to remote")?;

    let default_branch = remote.default_branch()?;
    let default_branch = default_branch.as_str().ok_or(anyhow::anyhow!(
        "Default branch is not a valid string, what the fuck"
    ))?;
    let branch_name = format!("mdb-{}-{}", base_sha, head_sha);
    let base_commit = {
        remote
            .fetch(&[default_branch], None, None)
            .context("Fetching base")?;
        let fetch_head = repo.find_reference("FETCH_HEAD")?;

        let base_commit = repo
            .reference_to_annotated_commit(&fetch_head)
            .context("Getting commit from FETCH_HEAD")?;

        let mut origin_ref = repo.resolve_reference_from_short_name(default_branch)?;

        origin_ref
            .set_target(base_commit.id(), "Fast forwarding origin ref")
            .context("Setting default branch to FETCH_HEAD's commit")?;

        repo.set_head(origin_ref.name().unwrap())
            .context("Setting HEAD")?;

        let base_commit = repo.find_commit(base_id).context("Finding base commit")?;
        base_commit
    };
    let diffs = {
        remote
            .fetch(
                &[extra_branch],
                Some(FetchOptions::new().prune(git2::FetchPrune::On)),
                None,
            )
            .context("Fetching head")?;

        let fetch_head = repo.find_reference("FETCH_HEAD")?;

        let head_commit = repo
            .reference_to_annotated_commit(&fetch_head)
            .context("Getting commit from FETCH_HEAD")?;

        let mut head_branch = repo
            .branch_from_annotated_commit(&branch_name, &head_commit, false)
            .context("Creating branch from FETCH_HEAD's commit")?
            .into_reference();

        repo.set_head(head_branch.name().unwrap())?;

        let head_commit = repo.find_commit(head_id).context("Finding head commit")?;

        let diffs = repo
            .diff_tree_to_tree(Some(&base_commit.tree()?), Some(&head_commit.tree()?), None)
            .context("Grabbing diffs")?;

        head_branch.delete().context("Cleaning up branch")?;
        diffs
    };

    repo.reset(
        &base_commit.as_object(),
        git2::ResetType::Hard,
        Some(
            git2::build::CheckoutBuilder::default()
                .force()
                .remove_untracked(true)
                .remove_ignored(true),
        ),
    )
    .context("Resetting to base commit")?;

    remote.disconnect().context("Disconnecting from remote")?;

    Ok(diffs)
}

pub fn with_changes_and_dir<T>(
    diff: &Diff,
    repo: &Repository,
    repodir: &Path,
    f: impl FnOnce() -> Result<T>,
) -> Result<T> {
    with_repo_dir(repodir, || {
        repo.apply(diff, git2::ApplyLocation::WorkDir, None)
            .context("Applying changes")?;
        let result = f();
        repo.checkout_head(Some(CheckoutBuilder::new().force()))
            .context("Resetting to HEAD")?;
        result
    })
}

pub fn clone_repo(url: &str, dir: &Path) -> Result<()> {
    git2::Repository::clone(url, dir.as_os_str()).context("Cloning repo")?;
    Ok(())
}
