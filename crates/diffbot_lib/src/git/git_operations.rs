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
    remote
        .fetch(&[default_branch], None, None)
        .context("Fetching base")?;
    remote
        .fetch(
            &[extra_branch],
            Some(FetchOptions::new().prune(git2::FetchPrune::On)),
            None,
        )
        .context("Fetching head")?;

    let actual_commit = repo
        .find_commit(base_id)
        .context("Looking for base commit")?;
    let remote_commit = repo
        .find_commit(head_id)
        .context("Looking for head commit")?;

    repo.reset(
        actual_commit.as_object(),
        git2::ResetType::Hard,
        Some(git2::build::CheckoutBuilder::default().force()),
    )
    .context("Resetting to commit")?;

    let diffs = repo
        .diff_tree_to_tree(
            Some(&actual_commit.tree()?),
            Some(&remote_commit.tree()?),
            None,
        )
        .context("Grabbing diffs")?;

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
