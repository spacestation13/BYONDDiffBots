use eyre::{Context, Result};
use secrecy::{Secret, ExposeSecret};
use std::path::Path;

use git2::{build::{CheckoutBuilder, RepoBuilder}, FetchOptions, Repository, RemoteCallbacks, Cred};

pub fn fetch_and_get_branches<'a>(
    base_sha: &str,
    head_sha: &str,
    repo: &'a git2::Repository,
    head_branch_name: &str,
    base_branch_name: &str,
    repo_token: Secret<String>,
) -> Result<(git2::Reference<'a>, git2::Reference<'a>)> {
    let base_id = git2::Oid::from_str(base_sha).wrap_err("Parsing base sha")?;
    let head_id = git2::Oid::from_str(head_sha).wrap_err("Parsing head sha")?;

    let mut remote = repo.find_remote("origin")?;

    let mut fetch_options = create_fetch_options_for_token(repo_token);
    fetch_options.prune(git2::FetchPrune::On);

    remote
        .fetch(
            &[base_branch_name],
            Some(&mut fetch_options),
            None,
        )
        .wrap_err("Fetching base")?;
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

    let base_branch = repo
        .resolve_reference_from_short_name(base_branch_name)
        .wrap_err("Getting the base reference")?;

    remote
        .fetch(
            &[head_branch_name],
            Some(&mut fetch_options),
            None,
        )
        .wrap_err("Fetching head")?;

    let fetch_head = repo
        .find_reference("FETCH_HEAD")
        .wrap_err("Getting FETCH_HEAD")?;

    let head_name = format!("mdb-pull-{base_sha}-{head_sha}");

    let mut head_branch = repo
        .branch_from_annotated_commit(
            &head_name,
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

    let head_branch = repo
        .resolve_reference_from_short_name(&head_name)
        .wrap_err("Getting the head reference")?;

    remote.disconnect().wrap_err("Disconnecting from remote")?;

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

    Ok((base_branch, head_branch))
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

fn create_fetch_options_for_token(repo_token: Secret<String>) -> FetchOptions<'static>{
    let mut callbacks = RemoteCallbacks::new();
    callbacks.credentials(move |_url, _username_from_url, _allowed_types| {
        Cred::userpass_plaintext(repo_token.expose_secret(), "")
    });

    let mut fetch_options = git2::FetchOptions::new();
    fetch_options.remote_callbacks(callbacks);
    fetch_options
}

pub fn clone_repo(url: &str, dir: &Path, repo_token: Secret<String>) -> Result<()> {
    let mut builder = RepoBuilder::new();
    builder.fetch_options(create_fetch_options_for_token(repo_token));

    builder.clone(url, dir).wrap_err("Cloning repo")?;
    Ok(())
}

/* local testing
#[test]
fn test_private_clone(){
    clone_repo("https://github.com/Cyberboss/tgstation-private-test", &Path::new("S:/garbage/tgtest"), Secret::new("lol".to_string())).unwrap();
}

#[test]
fn test_private_fetch(){
    let repo = git2::Repository::open("S:/garbage/tgtest").unwrap();
    fetch_and_get_branches("140c79189849ea616f09b3484f8930211d3705cd", "a34219208f6526d01d88c9fe02cc08554fe29dda", &repo, "TestPRForMDB", "master", Secret::new("lol".to_string())).unwrap();
}
 */
