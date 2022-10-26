pub fn init_logger() -> eyre::Result<()> {
    use simplelog::*;

    #[cfg(not(debug_assertions))]
    TermLogger::init(
        LevelFilter::Info,
        Config::default(),
        TerminalMode::Stdout,
        ColorChoice::Always,
    )?;

    #[cfg(debug_assertions)]
    TermLogger::init(
        LevelFilter::Trace,
        Config::default(),
        TerminalMode::Stdout,
        ColorChoice::Always,
    )?;

    Ok(())
}

pub fn init_file_logger(filename: &str) -> eyre::Result<()> {
    use simplelog::*;

    #[cfg(not(debug_assertions))]
    WriteLogger::init(
        LevelFilter::Info,
        Config::default(),
        std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(filename)
            .expect("Opening log file for write failed"),
    )?;

    #[cfg(debug_assertions)]
    WriteLogger::init(
        LevelFilter::Trace,
        Config::default(),
        std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(filename)
            .expect("Opening log file for write failed"),
    )?;

    Ok(())
}
