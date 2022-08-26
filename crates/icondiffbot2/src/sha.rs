use anyhow::{Context, Result};
use diffbot_lib::{
    github::{github_api::download_url, github_types::ChangeType},
    job::types::Job,
};
use dmm_tools::dmi::IconFile;
use std::{
    collections::hash_map::DefaultHasher,
    hash::{Hash, Hasher},
};

#[derive(Debug)]
pub struct IconFileWithName {
    pub full_name: String,
    pub sha: String,
    pub hash: u64,
    pub icon: IconFile,
}

pub fn status_to_sha<'a>(job: &'a Job, status: &ChangeType) -> (Option<&'a str>, Option<&'a str>) {
    match status {
        ChangeType::Added => (None, Some(&job.head.sha)),
        ChangeType::Deleted => (Some(&job.base.sha), None),
        ChangeType::Modified => (Some(&job.base.sha), Some(&job.head.sha)),
        _ => (None, None),
    }
}

pub async fn sha_to_iconfile(
    job: &Job,
    filename: &str,
    sha: (Option<&str>, Option<&str>),
) -> Result<(Option<IconFileWithName>, Option<IconFileWithName>)> {
    Ok((
        get_if_exists(job, filename, sha.0).await?,
        get_if_exists(job, filename, sha.1).await?,
    ))
}

#[tracing::instrument]
pub async fn get_if_exists(
    job: &Job,
    filename: &str,
    sha: Option<&str>,
) -> Result<Option<IconFileWithName>> {
    if let Some(sha) = sha {
        let raw = download_url(&job.installation, &job.base.repo, filename, sha)
            .await
            .with_context(|| format!("Failed to download file {:?}", filename))?;

        let mut hasher = DefaultHasher::new();
        raw.hash(&mut hasher);
        let hash = hasher.finish();

        Ok(Some(IconFileWithName {
            full_name: filename.to_string(),
            sha: sha.to_string(),
            hash,
            icon: IconFile::from_bytes(&raw)
                .with_context(|| format!("IconFile::from_bytes failed for {:?}", filename))?,
        }))
    } else {
        Ok(None)
    }
}
