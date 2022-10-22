use super::github_types::{ChangeType, FileDiff};
use eyre::Result;
use octocrab::models::InstallationId;
use serde::Deserialize;

/*
  Query:
{
  repository(owner: "tgstation", name: "tgstation") {
    pullRequest(number: 69416) {
      files(first: 100, after: "MjAw") {
        edges {
          cursor
          node {
            path
            changeType
          }
        }
      }
    }
  }
}
  Sample Response:
{
  "data": {
    "repository": {
      "pullRequest": {
        "files": {
          "edges": [
            {
              "cursor": "MjAx",
              "node": {
                "path": "code/modules/projectiles/guns/energy/kinetic_accelerator.dm",
                "changeType": "MODIFIED"
              }
            },
            {
              "cursor": "MjAy",
              "node": {
                "path": "code/modules/projectiles/guns/energy/laser.dm",
                "changeType": "MODIFIED"
              }
            },
          ]
        }
      }
    }
  }
}
*/

#[derive(Deserialize)]
struct QueryData {
    data: Data,
}

#[derive(Deserialize)]
struct Data {
    repository: Reposit,
}

#[derive(Deserialize)]
struct Reposit {
    #[serde(rename(deserialize = "pullRequest"))]
    pull_request: PullRequest,
}

#[derive(Deserialize)]
struct PullRequest {
    files: Edges,
}

#[derive(Deserialize)]
struct Edges {
    edges: Vec<Edge>,
}

#[derive(Deserialize)]
struct Edge {
    cursor: String,
    node: Node,
}

#[derive(Deserialize)]
struct Node {
    path: String,
    #[serde(rename(deserialize = "changeType"))]
    change_type: String,
}

pub async fn get_pull_files<I: Into<InstallationId>>(
    (user, repo): (String, String),
    installation: I,
    pull: &super::github_types::PullRequest,
) -> Result<Vec<FileDiff>> {
    let crab = octocrab::instance().installation(installation.into());

    let mut cursor = "".to_string();

    let mut ret = vec![];

    loop {
        let queried: QueryData = crab
            .graphql(&format!(
                "
query {{ 
  repository(owner:\"{}\", name:\"{}\") {{
    pullRequest(number:{}) {{
      files(first:100, after:\"{}\") {{
        edges {{
          cursor
          node {{
            path
            changeType
          }}
        }}
      }}
    }}
  }}
}}",
                user, repo, pull.number, cursor
            ))
            .await?;

        if queried.data.repository.pull_request.files.edges.is_empty() {
            break;
        }

        cursor = match queried.data.repository.pull_request.files.edges.last() {
            Some(edge) => edge.cursor.clone(),
            None => "".to_owned(),
        };
        ret.extend(
            queried
                .data
                .repository
                .pull_request
                .files
                .edges
                .into_iter()
                .map(|item| {
                    let status = match item.node.change_type.as_str() {
                        "ADDED" => ChangeType::Added,
                        "CHANGED" => ChangeType::Changed,
                        "COPIED" => ChangeType::Copied,
                        "DELETED" => ChangeType::Deleted,
                        "MODIFIED" => ChangeType::Modified,
                        "RENAMED" => ChangeType::Renamed,
                        _ => unreachable!("changeType for graphql query not covered!"),
                    };
                    FileDiff {
                        status,
                        filename: item.node.path,
                    }
                }),
        );
    }
    Ok(ret)
}
/*
#[derive(Deserialize)]
pub struct CheckRun {}
pub async fn create_check_run<I: Into<InstallationId>>(
    installation: I,
    repo_id: &str,
    sha: &str,
) -> Result<CheckRun> {
    let crab = octocrab::instance().installation(installation.into());
    let run: CheckRun = crab
        .graphql(&format!(
            "
mutation {{
  createCheckRun(input: {{
    clientMutationId:\"MAPDIFF\"
    repositoryId: \"{}\"
    name:\"MapDiffBot2\"
    headSha: \"{}\"
  }}) {{
    checkRun {{
      id
    }}
  }}
}}
        ",
            repo_id, sha
        ))
        .await?;
    Ok(CheckRun {})
}
*/
