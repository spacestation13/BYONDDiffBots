use anyhow::Result;
use std::path::Path;

use git2::{Diff, Repository};

pub fn with_repo_dir<T>(repo: &Path, f: impl FnOnce() -> Result<T>) -> Result<T> {
    let current_dir = std::env::current_dir()?;
    std::env::set_current_dir(repo)?;
    let result = f();
    std::env::set_current_dir(current_dir)?;
    result
}

pub fn fast_forward_to_head(head_sha: &str, repo: &Repository) -> Result<()> {
    let id = git2::Oid::from_str(head_sha)?;
    let mut remote = repo.find_remote("origin")?;
    let default_branch = remote.default_branch()?;
    remote.fetch(
        &[default_branch.as_str().ok_or(anyhow::anyhow!(
            "Default branch is not a valid string, what the fuck"
        ))?],
        None,
        None,
    )?;
    let fetch_head = repo.find_reference("FETCH_HEAD")?;
    let actual_tree = fetch_head.peel_to_tree()?;
    let entry = actual_tree
        .get_id(id)
        .ok_or(anyhow::anyhow!("Cannot find commit from fetched head"))?;
    repo.reset(&entry.to_object(&repo)?, git2::ResetType::Hard, None)?;

    Ok(())
}

pub fn with_deltas<T>(diff: &Diff, repo: &Repository, f: impl FnOnce() -> Result<T>) -> Result<T> {
    repo.apply(diff, git2::ApplyLocation::WorkDir, None)?;
    let result = f();
    let head = repo.head()?;
    repo.reset(
        head.peel_to_tree()?.as_object(),
        git2::ResetType::Hard,
        None,
    )?;
    result
}
