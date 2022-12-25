use crate::github::{github_api::CheckRun, github_types::Output};

pub async fn handle_output<S: AsRef<str>>(output: Vec<Output>, check_run: CheckRun, name: S) {
    match output.len() {
        0 => {
            let _ = check_run
                .mark_succeeded(Output {
                    title: "No relevant changes",
                    summary: "No relevant changes detected, have metadatas been modified?"
                        .to_owned(),
                    text: "".to_owned(),
                })
                .await;
        }
        1 => {
            let res = check_run
                .mark_succeeded(output.into_iter().next().unwrap())
                .await;
            if res.is_err() {
                let _ = check_run
                    .mark_failed(&format!("Failed to upload job output: {:?}", res))
                    .await;
            }
        }
        len => {
            for (idx, item) in output.into_iter().enumerate() {
                match idx {
                    0 => {
                        let _ = check_run
                            .rename(&format!("{} (1/{})", name.as_ref(), len))
                            .await;
                        let res = check_run.mark_succeeded(item).await;
                        if res.is_err() {
                            let _ = check_run
                                .mark_failed(&format!("Failed to upload job output: {:?}", res))
                                .await;
                            return;
                        }
                    }
                    _ => {
                        if let Ok(check) = check_run
                            .duplicate(&format!("{} ({}/{})", name.as_ref(), idx + 1, len))
                            .await
                        {
                            let res = check.mark_succeeded(item).await;
                            if res.is_err() {
                                let _ = check_run
                                    .mark_failed(&format!("Failed to upload job output: {:?}", res))
                                    .await;
                                return;
                            }
                        }
                    }
                }
            }
        }
    }
}
