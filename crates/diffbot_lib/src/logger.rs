use log::Level;

fn get_log_level(log_level: &str) -> Level {
    match log_level {
        "trace" => Level::Trace,
        "debug" => Level::Debug,
        "info" => Level::Info,
        "warn" => Level::Warn,
        "error" => Level::Error,
        _ => Level::Error,
    }
}

pub fn init_logger(log_level: &str) -> eyre::Result<()> {
    simple_logger::init_with_level(get_log_level(log_level))?;

    Ok(())
}
/*
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
*/
