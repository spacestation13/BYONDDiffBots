use std::{
    collections::{hash_map::DefaultHasher, HashMap},
    hash::{Hash, Hasher},
};

use anyhow::{Context, Result};
use diffbot_lib::{
    github::{
        github_api::download_url,
        github_types::{CheckOutputs, ModifiedFileStatus, Output},
    },
    job::types::Job,
};
use dmm_tools::dmi::IconFile;

pub fn status_to_sha(job: &Job, status: ModifiedFileStatus) -> (Option<&str>, Option<&str>) {
    match status {
        ModifiedFileStatus::Added => (None, Some(&job.head.sha)),
        ModifiedFileStatus::Removed => (Some(&job.base.sha), None),
        ModifiedFileStatus::Modified => (Some(&job.base.sha), Some(&job.head.sha)),
        ModifiedFileStatus::Renamed => (None, None),
        ModifiedFileStatus::Copied => (None, None),
        ModifiedFileStatus::Changed => (None, None), // TODO: look up what this is
        ModifiedFileStatus::Unchanged => (None, None),
    }
}

pub struct IconFileWithName {
    pub full_name: String,
    pub sha: String,
    pub hash: u64,
    pub icon: IconFile,
}

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
            icon: IconFile::from_raw(raw)
                .with_context(|| format!("IconFile::from_raw failed for {:?}", filename))?,
        }))
    } else {
        Ok(None)
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

#[derive(Default, Debug)]
pub struct OutputTableBuilder<'a> {
    map: HashMap<&'a str, (String, Vec<String>)>,
}

impl<'a> OutputTableBuilder<'a> {
    pub fn new() -> Self {
        Default::default()
    }

    pub fn insert(
        &mut self,
        k: &'a str,
        v: (String, Vec<String>),
    ) -> Option<(String, Vec<String>)> {
        self.map.insert(k, v)
    }

    pub async fn build(&self) -> Result<CheckOutputs> {
        // TODO: Make this not shit
        let mut file_names: HashMap<&str, u32> = HashMap::new();
        let mut details: Vec<(String, &str, String)> = Vec::new();
        let mut current_table = String::new();

        for (file_name, (change_type, states)) in self.map.iter() {
            let entry = file_names.entry(file_name).or_insert(0);

            for state in states {
                // A little extra buffer room for the <detail> block
                if current_table.len() + state.len() > 55_000 {
                    details.push((
                        format!("{} ({})", file_name, *entry),
                        change_type,
                        std::mem::take(&mut current_table),
                    ));
                    *entry += 1;
                }
                current_table.push_str(state.as_str());
                current_table.push('\n');
            }

            if !current_table.is_empty() {
                details.push((
                    format!("{} ({})", file_name, *entry),
                    change_type,
                    std::mem::take(&mut current_table),
                ));
                *entry += 1;
            }
        }

        let mut chunks: Vec<Output> = Vec::new();
        let mut current_output_text = String::new();

        for (file_name, change_type, table) in details.iter() {
            // TODO: use an <img> tag so i can set a style that upscales 32x32 to 64x64
            // and sets all the browser flags for nearest neighbor scaling
            let diff_block = format!(
                include_str!(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/templates/diff_details.txt"
                )),
                filename = file_name,
                table = table,
                typ = change_type,
            );

            if current_output_text.len() + diff_block.len() > 60_000 {
                chunks.push(Output {
                    title: "Icon difference rendering".to_owned(),
                    summary: "*This is still a beta. Please file any issues [here](https://github.com/spacestation13/BYONDDiffBots/).*\n\nIcons with diff:".to_owned(),
                    text: std::mem::take(&mut current_output_text)
                });
            }

            current_output_text.push_str(&diff_block);
        }

        if !current_output_text.is_empty() {
            chunks.push(Output {
                title: "Icon difference rendering".to_owned(),
                summary: "*This is still a beta. Please file any issues [here](https://github.com/spacestation13/BYONDDiffBots/).*\n\nIcons with diff:".to_owned(),
                text: std::mem::take(&mut current_output_text)
            });
        }

        let first = chunks.drain(0..1).next().unwrap();
        if !chunks.is_empty() {
            Ok(CheckOutputs::Many(first, chunks))
        } else {
            Ok(CheckOutputs::One(first))
        }
    }
}
