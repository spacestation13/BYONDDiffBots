use crate::github::{github_api::CheckRun, github_types::Output};

use eyre::Result;

pub async fn handle_output<S: AsRef<str>>(
    output: Vec<Output>,
    check_run: &CheckRun,
    name: S,
) -> Result<()> {
    match output.len() {
        0 => {
            check_run
                .mark_succeeded(Output {
                    title: "No relevant changes",
                    summary: "No relevant changes detected, have metadatas been modified?"
                        .to_owned(),
                    text: "".to_owned(),
                })
                .await?
        }
        1 => {
            check_run
                .mark_succeeded(output.into_iter().next().unwrap())
                .await?;
        }
        len => {
            for (idx, item) in output.into_iter().enumerate() {
                match idx {
                    0 => {
                        check_run
                            .rename(&format!("{} (1/{len})", name.as_ref()))
                            .await?;
                        check_run.mark_succeeded(item).await?
                    }
                    _ => {
                        let check = check_run
                            .duplicate(&format!("{} ({}/{len})", name.as_ref(), idx + 1))
                            .await?;
                        check.mark_succeeded(item).await?
                    }
                };
            }
        }
    }
    Ok(())
}
