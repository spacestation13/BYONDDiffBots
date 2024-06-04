use eyre::{Context, Result};
use std::path::Path;

use git2::{build::CheckoutBuilder, FetchOptions, Repository};

pub fn fetch_and_get_branches<'a>(
    base_sha: &str,
    head_sha: &str,
    repo: &'a git2::Repository,
    head_branch_name: &str,
    base_branch_name: &str,
) -> Result<(git2::Reference<'a>, git2::Reference<'a>)> {
    let base_id = git2::Oid::from_str(base_sha).wrap_err("Parsing base sha")?;
    let head_id = git2::Oid::from_str(head_sha).wrap_err("Parsing head sha")?;

    let mut remote = repo.find_remote("origin")?;

    remote
        .connect(git2::Direction::Fetch)
        .wrap_err("Connecting to remote")?;

    remote
        .fetch(
            &[base_branch_name, head_branch_name],
            Some(FetchOptions::new().prune(git2::FetchPrune::On)),
            None,
        )
        .wrap_err("Fetching base and head")?;

    remote.disconnect().wrap_err("Disconnecting from remote")?;

    let fetch_head = repo
        .find_reference("FETCH_HEAD")
        .wrap_err("Getting FETCH_HEAD")?;

    let base_commit = repo
        .reference_to_annotated_commit(&fetch_head)
        .wrap_err("Getting commit from FETCH_HEAD")?;

    if let Some(branch) = repo
        .find_branch(base_branch_name, git2::BranchType::Local)
        .ok()
        .and_then(|branch| branch.is_head().then_some(branch))
    {
        branch
            .into_reference()
            .set_target(base_commit.id(), "Fast forwarding current ref")
            .wrap_err("Setting base reference to FETCH_HEAD's commit")?;
    } else {
        repo.branch_from_annotated_commit(base_branch_name, &base_commit, true)
            .wrap_err("Setting a new base branch to FETCH_HEAD's commit")?;
    }

    repo.set_head(
        repo.resolve_reference_from_short_name(base_branch_name)?
            .name()
            .unwrap(),
    )
    .wrap_err("Setting HEAD to base")?;

    let commit = match repo.find_commit(base_id).wrap_err("Finding base commit") {
        Ok(commit) => commit,
        Err(_) => repo.head()?.peel_to_commit()?,
    };

    repo.resolve_reference_from_short_name(base_branch_name)?
        .set_target(commit.id(), "Setting default branch to the correct commit")?;

    let fetch_head = repo
        .find_reference("FETCH_HEAD")
        .wrap_err("Getting FETCH_HEAD")?;

    let head_branch_name = format!("mdb-pull-{base_sha}-{head_sha}");

    let mut head_branch = repo
        .branch_from_annotated_commit(
            &head_branch_name,
            &repo.reference_to_annotated_commit(&fetch_head)?,
            true,
        )
        .wrap_err("Creating branch")?
        .into_reference();

    repo.set_head(head_branch.name().unwrap())
        .wrap_err("Setting HEAD to head")?;

    let head_commit = match repo.find_commit(head_id).wrap_err("Finding head commit") {
        Ok(commit) => commit,
        Err(_) => repo.head()?.peel_to_commit()?,
    };

    head_branch.set_target(
        head_commit.id(),
        "Setting head branch to the correct commit",
    )?;

    merge_base_into_head(base_branch_name, &head_branch_name, repo).wrap_err("Merging")?;

    repo.set_head(
        repo.resolve_reference_from_short_name(base_branch_name)?
            .name()
            .unwrap(),
    )
    .wrap_err("Setting head to default branch")?;

    repo.checkout_head(Some(
        CheckoutBuilder::default()
            .force()
            .remove_ignored(true)
            .remove_untracked(true),
    ))
    .wrap_err("Resetting to base commit")?;

    Ok((
        repo.resolve_reference_from_short_name(base_branch_name)
            .wrap_err("Getting the base reference")?,
        repo.resolve_reference_from_short_name(&head_branch_name)
            .wrap_err("Getting the head reference")?,
    ))
}

fn merge_base_into_head(base: &str, head: &str, repo: &Repository) -> Result<()> {
    repo.set_head(
        repo.resolve_reference_from_short_name(head)?
            .name()
            .unwrap(),
    )?;
    repo.checkout_head(Some(
        CheckoutBuilder::default()
            .force()
            .remove_ignored(true)
            .remove_untracked(true),
    ))
    .wrap_err("Resetting repo to head")?;

    let head_commit = repo.head()?.peel_to_commit()?;

    let base_branch = repo.resolve_reference_from_short_name(base)?;

    if let Err(e) = repo
        .merge(
            &[&repo.reference_to_annotated_commit(&base_branch)?],
            Some(
                git2::MergeOptions::default()
                    .ignore_whitespace(true)
                    .fail_on_conflict(false)
                    .file_favor(git2::FileFavor::Theirs),
            ),
            Some(
                CheckoutBuilder::default()
                    .force()
                    .remove_ignored(true)
                    .remove_untracked(true),
            ),
        )
        .wrap_err("Trying to merge base into head")
    {
        repo.cleanup_state()?;
        repo.set_head(
            repo.resolve_reference_from_short_name(base)?
                .name()
                .unwrap(),
        )?;
        repo.checkout_head(Some(
            CheckoutBuilder::default()
                .force()
                .remove_ignored(true)
                .remove_untracked(true),
        ))
        .wrap_err("Resetting to base commit")?;
        return Err(e);
    };

    let treeoid = repo.index()?.write_tree()?;

    let destination_commit = repo.head()?.peel_to_commit()?;

    let merge_commit = repo.commit(
        Some("HEAD"),
        &head_commit.author(),
        &head_commit.author(),
        "MAPDIFFBOT: MERGING BASE INTO HEAD",
        &repo.find_tree(treeoid)?,
        &[&head_commit, &destination_commit],
    )?;
    repo.cleanup_state()?;

    repo.resolve_reference_from_short_name(head)?
        .set_target(merge_commit, "Setting head to the merge commit")?;
    Ok(())
}

pub fn clean_up_references(repo: &Repository, branch: &str) -> Result<()> {
    repo.set_head(
        repo.resolve_reference_from_short_name(branch)?
            .name()
            .unwrap(),
    )
    .wrap_err("Setting head")?;
    repo.checkout_head(Some(
        CheckoutBuilder::new()
            .force()
            .remove_ignored(true)
            .remove_untracked(true),
    ))
    .wrap_err("Checkout to head")?;
    let mut references = repo.references().wrap_err("Getting all references")?;
    let references = references
        .names()
        .filter_map(move |reference| {
            (reference.as_ref().ok()?.contains("pull-"))
                .then(move || reference.ok())
                .flatten()
        })
        .map(|item| item.to_owned())
        .collect::<Vec<_>>();

    for refname in references {
        let mut reference = repo
            .find_reference(&refname)
            .wrap_err("Looking for ref to delete")?;
        reference.delete().wrap_err("Deleting reference")?;
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
    git2::Repository::clone(url, dir.as_os_str()).wrap_err("Cloning repo")?;
    Ok(())
}
