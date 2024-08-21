use diffbot_lib::github::github_types::Repository;
use eyre::Result;
use octocrab::models::repos::Content;
use octocrab::models::InstallationId;
use secrecy::ExposeSecret;

//https://url.spec.whatwg.org/#c0-control-percent-encode-set
const PATH_ENCODING: &percent_encoding::AsciiSet = &percent_encoding::CONTROLS
    //query
    .add(b' ')
    .add(b'"')
    .add(b'<')
    .add(b'>')
    .add(b'`')
    //path
    .add(b'?')
    .add(b'}')
    .add(b'{');

async fn find_content<S: AsRef<str>>(
    installation: &InstallationId,
    repo: &Repository,
    filename: S,
    commit: S,
) -> Result<Content> {
    let (owner, repo) = repo.name_tuple();
    let items = octocrab::instance()
        .installation(*installation)
        .repos(owner, repo)
        .get_content()
        .path(
            percent_encoding::percent_encode(filename.as_ref().as_bytes(), PATH_ENCODING)
                .to_string(),
        )
        .r#ref(commit.as_ref())
        .send()
        .await?
        .take_items();

    if items.len() > 1 {
        return Err(eyre::eyre!("Directory given to find_content"));
    }

    items
        .into_iter()
        .next()
        .ok_or_else(|| eyre::eyre!("No content was found"))
}

pub async fn download_url<S: AsRef<str>>(
    installation: &InstallationId,
    repo: &Repository,
    filename: S,
    commit: S,
    client: reqwest::Client,
) -> Result<Vec<u8>> {
    let target = find_content(installation, repo, filename, commit).await?;

    let download_url = target
        .download_url
        .as_ref()
        .ok_or_else(|| eyre::eyre!("No download URL given by GitHub"))?;
    let (_, token) = octocrab::instance()
        .installation_and_token(*installation)
        .await?;
    let response = client
        .get(download_url)
        .bearer_auth(token.expose_secret())
        .send()
        .await?;
    Ok(response.bytes().await?.to_vec())
}
