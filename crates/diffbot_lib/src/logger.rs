use tracing::level_filters::LevelFilter;

fn get_log_level(log_level: &str) -> LevelFilter {
    match log_level {
        "trace" => LevelFilter::TRACE,
        "debug" => LevelFilter::DEBUG,
        "info" => LevelFilter::INFO,
        "warn" => LevelFilter::WARN,
        "error" => LevelFilter::ERROR,
        _ => LevelFilter::ERROR,
    }
}

pub fn init_logger(log_level: &str) -> eyre::Result<()> {
    tracing_subscriber::fmt()
        .with_level(true)
        .with_line_number(true)
        .with_file(true)
        .without_time()
        .with_max_level(get_log_level(log_level))
        .init();
    Ok(())
}
