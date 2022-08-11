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

    let fetch_head = repo.find_reference("FETCH_HEAD")?;
    let annotated = repo.reference_to_annotated_commit(&fetch_head)?;

    let refname = format!("refs/heads/{}", default_branch);

    let mut head_reference = repo.find_reference(&refname)?;

    let name = match head_reference.name() {
        Some(s) => s.to_string(),
        None => String::from_utf8_lossy(head_reference.name_bytes()).to_string(),
    };

    head_reference.set_target(annotated.id(), "Fast forwarding")?;
    repo.set_head(&name)?;
    repo.checkout_head(Some(git2::build::CheckoutBuilder::default().force()))?;

    if let Ok(actual_commit) = repo.find_commit(id) {
        repo.reset(
            actual_commit.as_object(),
            git2::ResetType::Hard,
            Some(git2::build::CheckoutBuilder::default().force()),
        )?;
    }

    Ok(())
}

pub fn with_deltas<T>(diff: &Diff, repo: &Repository, f: impl FnOnce() -> Result<T>) -> Result<T> {
    repo.apply(diff, git2::ApplyLocation::WorkDir, None)?;
    let result = f();
    repo.checkout_head(Some(CheckoutBuilder::new().force()))?;
    result
}
