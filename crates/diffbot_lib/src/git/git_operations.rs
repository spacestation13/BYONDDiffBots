use anyhow::Result;
use std::{path::Path, process::Command};

use crate::github::github_types::Repository;

pub fn with_repo_dir<T>(repo: &Repository, f: impl FnOnce() -> Result<T>) -> Result<T> {
    let current_dir = std::env::current_dir()?;
    std::env::set_current_dir(Path::new(&format!("./repos/{}", repo.name)))?;
    let result = f();
    std::env::set_current_dir(current_dir)?;
    result
}

pub fn git_checkout(branch: &str) -> Result<(), std::io::Error> {
    Command::new("git")
        .args(["checkout", branch])
        .output()
        .map(|_| ())
}

pub fn with_checkout<T>(
    repo: &Repository,
    branch: &str,
    f: impl FnOnce() -> Result<T>,
) -> Result<T> {
    with_repo_dir(repo, || {
        git_checkout(branch)?;
        let result = f();
        git_checkout(repo.default_branch.as_ref().unwrap_or(&"master".to_owned()))?;
        result
    })
}
