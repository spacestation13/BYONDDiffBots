use anyhow::Result;
use std::path::Path;

use git2::{build::CheckoutBuilder, Diff, Repository};

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
    let default_branch = default_branch.as_str().ok_or(anyhow::anyhow!(
        "Default branch is not a valid string, what the fuck"
    ))?;
    remote.fetch(&[default_branch], None, None)?;

    let actual_commit = repo.find_commit(id)?;
    repo.reset(
        actual_commit.as_object(),
        git2::ResetType::Hard,
        Some(git2::build::CheckoutBuilder::default().force()),
    )?;

    Ok(())
}

pub fn with_deltas<T>(diff: &Diff, repo: &Repository, f: impl FnOnce() -> Result<T>) -> Result<T> {
    repo.apply(diff, git2::ApplyLocation::WorkDir, None)?;
    let result = f();
    repo.checkout_head(Some(CheckoutBuilder::new().force()))?;
    result
}
