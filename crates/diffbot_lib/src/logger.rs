use simplelog::*;

fn get_log_level(log_level: &str) -> LevelFilter {
    match log_level {
        "trace" => LevelFilter::Trace,
        "debug" => LevelFilter::Debug,
        "info" => LevelFilter::Info,
        "warn" => LevelFilter::Warn,
        "error" => LevelFilter::Error,
        _ => LevelFilter::Off,
    }
}

pub fn init_logger(log_level: &str) -> eyre::Result<()> {
    let level = get_log_level(log_level);

    TermLogger::init(
        level,
        Config::default(),
        TerminalMode::Stdout,
        ColorChoice::Always,
    )?;

    Ok(())
}

pub fn init_file_logger(log_level: &str, filename: &str) -> eyre::Result<()> {
    let level = get_log_level(log_level);

    WriteLogger::init(
        level,
        Config::default(),
        std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(filename)
            .expect("Opening log file for write failed"),
    )?;

    Ok(())
}
