use anyhow::Result;
use std::{path::Path, process::Command};

pub fn with_repo_dir<T>(repo: &str, f: impl FnOnce() -> Result<T>) -> Result<T> {
    eprintln!("with repo: {}", repo);
    let current_dir = std::env::current_dir()?;
    std::env::set_current_dir(Path::new(&format!("./repos/{}", repo)))?;
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

pub fn with_checkout<T>(repo: &str, branch: &str, f: impl FnOnce() -> Result<T>) -> Result<T> {
    eprintln!("with checkout: {} {}", branch, repo);
    with_repo_dir(repo, || {
        git_checkout(branch)?;
        let result = f();
        git_checkout("master")?;
        result
    })
}
