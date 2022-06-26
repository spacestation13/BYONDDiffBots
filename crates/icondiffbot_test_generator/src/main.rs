use clap::Parser;
use octocrab::models::pulls::FileDiff;

async fn get_pull_files(args: &Args) -> Result<Vec<FileDiff>, octocrab::Error> {
    let crab = octocrab::instance();
    let files = crab
        .pulls(&args.owner, &args.repo)
        .list_files(args.pull)
        .await?;
    crab.all_pages(files).await
}

#[derive(Debug, Parser)]
#[clap(author, version, about, long_about = None)]
struct Args {
    #[clap(value_parser)]
    owner: String,

    #[clap(value_parser)]
    repo: String,

    #[clap(value_parser)]
    pull: u64,
}

#[tokio::main]
async fn main() {
    let args = Args::parse();

    let files = get_pull_files(&args).await.unwrap();

    dbg!(files.len());
}
