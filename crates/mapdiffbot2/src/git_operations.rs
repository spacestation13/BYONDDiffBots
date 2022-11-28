use eyre::{Context, Result};
use std::path::Path;

use git2::{build::CheckoutBuilder, FetchOptions, Repository};

pub fn fetch_and_get_branches<'a>(
    base_sha: &str,
    head_sha: &str,
    repo: &'a git2::Repository,
    fetching_branch: &str,
    default_branch: &str,
) -> Result<(git2::Reference<'a>, git2::Reference<'a>)> {
    let base_id = git2::Oid::from_str(base_sha).context("Parsing base sha")?;
    let head_id = git2::Oid::from_str(head_sha).context("Parsing head sha")?;

    let mut remote = repo.find_remote("origin")?;

    remote
        .connect(git2::Direction::Fetch)
        .context("Connecting to remote")?;

    remote
        .fetch(
            &[default_branch],
            Some(FetchOptions::new().prune(git2::FetchPrune::On)),
            None,
        )
        .context("Fetching base")?;
    let fetch_head = repo
        .find_reference("FETCH_HEAD")
        .context("Getting FETCH_HEAD")?;

    let base_commit = repo
        .reference_to_annotated_commit(&fetch_head)
        .context("Getting commit from FETCH_HEAD")?;

    repo.resolve_reference_from_short_name(default_branch)?
        .set_target(base_commit.id(), "Fast forwarding origin ref")
        .context("Setting default branch to FETCH_HEAD's commit")?;

    repo.set_head(
        repo.resolve_reference_from_short_name(default_branch)?
            .name()
            .unwrap(),
    )
    .context("Setting HEAD to base")?;

    let commit = repo
        .find_commit(base_id)
        .context("Finding commit from base SHA")?;

    repo.resolve_reference_from_short_name(default_branch)?
        .set_target(commit.id(), "Setting default branch to the correct commit")?;

    let base_branch = repo
        .resolve_reference_from_short_name(default_branch)
        .context("Getting the base reference")?;

    remote
        .fetch(
            &[fetching_branch],
            Some(FetchOptions::new().prune(git2::FetchPrune::On)),
            None,
        )
        .context("Fetching head")?;

    let fetch_head = repo
        .find_reference("FETCH_HEAD")
        .context("Getting FETCH_HEAD")?;

    let head_name = format!("mdb-pull-{}-{}", base_sha, head_sha);

    let mut head_branch = repo
        .branch_from_annotated_commit(
            &head_name,
            &repo.reference_to_annotated_commit(&fetch_head)?,
            true,
        )
        .context("Creating branch")?
        .into_reference();

    repo.set_head(head_branch.name().unwrap())
        .context("Setting HEAD to head")?;

    let head_commit = repo.find_commit(head_id).context("Finding head commit")?;

    head_branch.set_target(
        head_commit.id(),
        "Setting head branch to the correct commit",
    )?;

    let head_branch = repo
        .resolve_reference_from_short_name(&head_name)
        .context("Getting the head reference")?;

    remote.disconnect().context("Disconnecting from remote")?;

    repo.set_head(
        repo.resolve_reference_from_short_name(default_branch)?
            .name()
            .unwrap(),
    )
    .context("Setting head to default branch")?;

    repo.checkout_head(Some(
        CheckoutBuilder::default()
            .force()
            .remove_ignored(true)
            .remove_untracked(true),
    ))
    .context("Resetting to base commit")?;

    Ok((base_branch, head_branch))
}

pub fn clean_up_references(repo: &Repository, default: &str) -> Result<()> {
    repo.set_head(default).context("Setting head")?;
    repo.checkout_head(Some(
        CheckoutBuilder::new()
            .force()
            .remove_ignored(true)
            .remove_untracked(true),
    ))
    .context("Checkout to head")?;
    let mut references = repo.references().context("Getting all references")?;
    let references = references
        .names()
        .filter_map(move |reference| {
            (reference.as_ref().ok()?.contains("pull") && reference.as_ref().ok()? != &default)
                .then(move || reference.ok())
                .flatten()
        })
        .map(|item| item.to_owned())
        .collect::<Vec<_>>();

    for refname in references {
        let mut reference = repo
            .find_reference(&refname)
            .context("Looking for ref to delete")?;
        reference.delete().context("Deleting reference")?;
    }
    Ok(())
}

pub fn with_checkout<T>(
    checkout_ref: &git2::Reference,
    repo: &Repository,
    f: impl FnOnce() -> Result<T>,
) -> Result<T> {
    repo.set_head(checkout_ref.name().unwrap())?;
    repo.checkout_head(Some(
        CheckoutBuilder::new()
            .force()
            .remove_ignored(true)
            .remove_untracked(true),
    ))?;
    f()
}

pub fn clone_repo(url: &str, dir: &Path) -> Result<()> {
    git2::Repository::clone(url, dir.as_os_str()).context("Cloning repo")?;
    Ok(())
}
