use anyhow::Result;
use diffbot_lib::github::github_types::{CheckOutputs, Output};
use std::collections::HashMap;

#[derive(Default, Debug)]
pub struct OutputTableBuilder<'a> {
    map: HashMap<&'a str, (&'static str, Vec<String>)>,
}

impl<'a> OutputTableBuilder<'a> {
    pub fn new() -> Self {
        Default::default()
    }

    #[tracing::instrument]
    pub fn insert(
        &mut self,
        k: &'a str,
        v: (&'static str, Vec<String>),
    ) -> Option<(&'static str, Vec<String>)> {
        self.map.insert(k, v)
    }

    #[tracing::instrument]
    pub fn build(&self) -> Result<CheckOutputs> {
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
                    title: "Icon difference rendering",
                    summary: "*This is still a beta. Please file any issues [here](https://github.com/spacestation13/BYONDDiffBots/).*\n\nIcons with diff:".to_string(),
                    text: std::mem::take(&mut current_output_text)
                });
            }

            current_output_text.push_str(&diff_block);
        }

        if !current_output_text.is_empty() {
            chunks.push(Output {
                title: "Icon difference rendering",
                summary: "*This is still a beta. Please file any issues [here](https://github.com/spacestation13/BYONDDiffBots/).*\n\nIcons with diff:".to_string(),
                text: std::mem::take(&mut current_output_text)
            });
        }
        Ok(chunks)
    }
}
