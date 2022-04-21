use ahash::RandomState;
use dmm_tools::dmi::Image;
use dmm_tools::render_passes::RenderPass;
use flume::Receiver;
use path_absolutize::*;
use std::collections::HashSet;
use std::path::Path;
use std::process::Command;
use std::sync::RwLock;

extern crate dreammaker as dm;

use dm::objtree::ObjectTree;
use dmm_tools::*;

use crate::github_types::{Branch, Empty, ModifiedFile, Output, Repository, UpdateCheckRun};
use crate::{job, CONFIG};

#[derive(Debug)]
struct BoundingBox {
    left: usize,
    bottom: usize,
    right: usize,
    top: usize,
}

fn with_repo_dir<T>(repo: &str, f: impl FnOnce() -> T) -> Result<T, std::io::Error> {
    eprintln!("with repo: {}", repo);
    let current_dir = std::env::current_dir()?;
    std::env::set_current_dir(Path::new(repo))?;
    let result = f();
    std::env::set_current_dir(current_dir)?;
    Ok(result)
}

fn git_checkout(repo: &str, branch: &str) -> Result<(), std::io::Error> {
    let output = Command::new("git")
        .args(["checkout", branch])
        .output()
        .expect("failed to execute process");
    Ok(())
}

fn with_checkout<T>(repo: &str, branch: &str, f: impl FnOnce() -> T) -> Result<T, std::io::Error> {
    eprintln!("with checkout: {} {}", branch, repo);
    with_repo_dir(repo, || {
        git_checkout(repo, branch)?;
        let result = f();
        git_checkout(repo, "master")?;
        Ok(result)
    })
    .unwrap()
}

fn get_diff_bounding_box(left_map: &dmm::Map, right_map: &dmm::Map, z_level: usize) -> BoundingBox {
    eprintln!("getting bounding box");
    use std::cmp::min;

    let left_dims = left_map.dim_xyz();
    let right_dims = right_map.dim_xyz();
    if left_dims != right_dims {
        println!("    different size: {:?} {:?}", left_dims, right_dims);
    }

    let mut rightmost = 0usize;
    let mut leftmost = left_dims.0;
    let mut topmost = 0usize;
    let mut bottommost = left_dims.1;

    for y in 0..min(left_dims.1, right_dims.1) {
        for x in 0..min(left_dims.0, right_dims.0) {
            let left_tile = &left_map.dictionary[&left_map.grid[(z_level, left_dims.1 - y - 1, x)]];
            let right_tile =
                &right_map.dictionary[&right_map.grid[(z_level, right_dims.1 - y - 1, x)]];
            if left_tile != right_tile {
                eprintln!("different tile: ({}, {}, {})", x + 1, y + 1, z_level + 1);
                if x < leftmost {
                    leftmost = x;
                }
                if x > rightmost {
                    rightmost = x;
                }
                if y < bottommost {
                    bottommost = y;
                }
                if y > topmost {
                    topmost = y;
                }
            }
        }
    }

    if leftmost > rightmost {
        leftmost = 0;
        rightmost = 1;
        bottommost = 0; // create a small bounding box for now if there are no changes
        topmost = 1;
    }

    BoundingBox {
        left: leftmost,
        bottom: bottommost,
        right: rightmost,
        top: topmost,
    }
}

struct MapDiff {
    base_map: dmm::Map,
    head_map: dmm::Map,
    bounding_box: BoundingBox,
}

fn get_map_diffs(base: &Branch, head: &str, files: &Vec<ModifiedFile>) -> Vec<MapDiff> {
    eprintln!("getting map diffs");
    let mut result = vec![];
    for file in files {
        let left_map = with_checkout(&base.repo.name, &base.name, || {
            dmm::Map::from_file(Path::new(&file.filename)).unwrap()
        })
        .unwrap();
        let right_map = with_checkout(&base.repo.name, head, || {
            dmm::Map::from_file(Path::new(&file.filename)).unwrap()
        })
        .unwrap();

        let bounding_box = get_diff_bounding_box(&left_map, &right_map, 0);

        result.push(MapDiff {
            base_map: left_map,
            head_map: right_map,
            bounding_box,
        });
    }
    result
}

#[derive(Default)]
struct Context {
    dm_context: dm::Context,
    objtree: ObjectTree,
    icon_cache: IconCache,
}

impl Context {
    fn objtree(&mut self, path: &Path) {
        let environment = match dm::detect_environment(path, dm::DEFAULT_ENV) {
            Ok(Some(found)) => found,
            _ => dm::DEFAULT_ENV.into(),
        };
        eprintln!("parsing {}", environment.display());

        if let Some(parent) = environment.parent() {
            self.icon_cache.set_icons_root(&parent);
        }

        self.dm_context.autodetect_config(&environment);
        let pp = match dm::preprocessor::Preprocessor::new(&self.dm_context, environment) {
            Ok(pp) => pp,
            Err(e) => {
                eprintln!("i/o error opening environment:\n{}", e);
                std::process::exit(1);
            }
        };
        let indents = dm::indents::IndentProcessor::new(&self.dm_context, pp);
        let parser = dm::parser::Parser::new(&self.dm_context, indents);
        self.objtree = parser.parse_object_tree();
    }
}

fn render_map(
    context: &Context,
    map: &dmm::Map,
    bb: &BoundingBox,
    errors: &RwLock<HashSet<String, RandomState>>,
    render_passes: &Vec<Box<dyn RenderPass>>,
) -> Result<Image, ()> {
    let bump = Default::default();
    let minimap_context = minimap::Context {
        objtree: &context.objtree,
        map: &map,
        level: map.z_level(0),
        min: (bb.left, bb.bottom),
        max: (bb.right, bb.top),
        render_passes: &render_passes,
        errors: &errors,
        bump: &bump,
    };
    println!("Generating map");
    eprintln!("{:?}", bb);
    minimap::generate(minimap_context, &context.icon_cache)
}

fn render_befores(
    base_context: &Context,
    diffs: &Vec<MapDiff>,
    render_passes: &Vec<Box<dyn RenderPass>>,
    output_dir: &Path,
) {
    eprintln!("rendering befores");
    for (idx, diff) in diffs.iter().enumerate() {
        eprintln!("rendering before");
        let before = render_map(
            &base_context,
            &diff.base_map,
            &diff.bounding_box,
            &Default::default(),
            render_passes,
        )
        .unwrap();

        let directory = format!("{}/{}", output_dir.display(), idx);

        eprintln!("Creating output directory: {}", directory);
        std::fs::create_dir_all(&directory).expect("Could not create path");

        eprintln!("saving images");
        before
            .to_file(format!("{}/{}", directory, "before.png").as_ref())
            .unwrap();
    }
}

fn render_afters(
    head_context: &Context,
    diffs: &Vec<MapDiff>,
    render_passes: &Vec<Box<dyn RenderPass>>,
    output_dir: &Path,
) {
    eprintln!("rendering afters");
    for (idx, diff) in diffs.iter().enumerate() {
        eprintln!("rendering after");
        let after = render_map(
            &head_context,
            &diff.head_map,
            &diff.bounding_box,
            &Default::default(),
            render_passes,
        )
        .unwrap();
        eprintln!("rendering after");

        let directory = format!("{}/{}", output_dir.display(), idx);

        eprintln!("Creating output directory: {}", directory);
        std::fs::create_dir_all(&directory).expect("Could not create path");

        eprintln!("saving images");
        after
            .to_file(format!("{}/{}", directory, "after.png").as_ref())
            .unwrap();
    }
}

fn render(
    repo: &Repository,
    base: &Branch,
    head: &Branch,
    files: &Vec<ModifiedFile>,
    output_dir: &Path,
    pull_request_number: u64,
) -> Result<(), ()> {
    let errors: RwLock<HashSet<String, RandomState>> = Default::default();

    eprintln!("Parsing base");
    let mut base_context = Context::default();
    base_context.objtree(&Path::new(&repo.name).absolutize().unwrap());

    let pull_branch = format!("mdb-{}-{}", base.sha, head.sha);
    let fetch_branch = format!("pull/{}/head:{}", pull_request_number, pull_branch);

    with_repo_dir(&base.repo.name, || {
        let output = Command::new("git")
            .args(["fetch", "origin", &fetch_branch])
            .output()
            .expect("failed to execute process");
    })
    .unwrap();

    eprintln!("Parsing head");
    let mut head_context = Context::default();
    with_checkout(&repo.name, &fetch_branch, || {
        head_context.objtree(&Path::new(".").absolutize().unwrap());
    })
    .unwrap();

    let render_passes = &dmm_tools::render_passes::configure(
        &base_context.dm_context.config().map_renderer, //TODO: also use render passes from head context
        "",
        "",
    );
    let diffs = get_map_diffs(&base, &pull_branch, files);

    render_befores(&base_context, &diffs, render_passes, output_dir);

    with_checkout(&repo.name, &pull_branch, || {
        render_afters(&head_context, &diffs, render_passes, output_dir);
    })
    .unwrap();
    Ok(())
}

pub async fn handle_jobs(job_receiver: Receiver<job::Job>) {
    eprintln!("Starting job handler");
    while let Ok(job) = job_receiver.recv_async().await {
        println!("Received job: {:#?}", job);

        let _: Empty = octocrab::instance()
            .installation(job.installation_id.into())
            .patch(
                format!(
                    "/repos/{repo}/check-runs/{check_run_id}",
                    repo = job.repository.full_name(),
                    check_run_id = job.check_run_id
                ),
                Some(&UpdateCheckRun {
                    conclusion: None,
                    completed_at: None,
                    status: Some("in_progress".to_string()),
                    name: None,
                    started_at: Some(chrono::Utc::now().to_rfc3339()),
                    output: None,
                }),
            )
            .await
            .expect("Could not update check run");

        let base = job.base;
        let head = job.head;
        let repo = format!("https://github.com/{}", base.repo.full_name());
        let branch = &base.name;
        let output = Command::new("git")
            .args(["clone", "--depth=1", "--branch", &branch, &repo])
            .output()
            .expect("failed to execute process");

        println!("{}", String::from_utf8_lossy(&output.stdout));
        println!("{}", String::from_utf8_lossy(&output.stderr));

        let non_abs_directory = format!("images/{}/{}", job.repository.id, job.check_run_id);
        let directory = Path::new(&non_abs_directory).absolutize().unwrap();
        let directory = directory.as_ref().to_str().unwrap();

        git_checkout(
            &base.repo.name,
            &job.repository
                .default_branch
                .clone()
                .unwrap_or("master".to_owned()),
        )
        .unwrap();

        render(
            &base.repo,
            &base,
            &head,
            &job.files,
            Path::new(directory),
            job.pull_request,
        )
        .unwrap();

        let conf = CONFIG.read().await;
        let file_url = &conf.as_ref().unwrap().file_hosting_url;

        let link_before = format!("{}/{}/0/before.png", file_url, non_abs_directory);
        let link_after = format!("{}/{}/0/after.png", file_url, non_abs_directory);

        let title = "Map renderings";
        let summary = "Maps with diff:";
        let text = format!(
            "\
<details>
	<summary>
	{}
	</summary>

|  Old  |      New      |  Difference  |
| :---: |     :---:     |    :---:     |
|![]({})|    ![]({})    |coming soon...|

</details>",
            job.files[0].filename, link_before, link_after
        );

        let output = Output {
            title: title.to_owned(),
            summary: summary.to_owned(),
            text,
        };

        let _: Empty = octocrab::instance()
            .installation(job.installation_id.into())
            .patch(
                format!(
                    "/repos/{repo}/check-runs/{check_run_id}",
                    repo = job.repository.full_name(),
                    check_run_id = job.check_run_id
                ),
                Some(&UpdateCheckRun {
                    conclusion: Some("success".to_string()),
                    completed_at: Some(chrono::Utc::now().to_rfc3339()),
                    status: None,
                    name: None,
                    started_at: None,
                    output: Some(output),
                }),
            )
            .await
            .expect("Could not update check run");
    }
}
