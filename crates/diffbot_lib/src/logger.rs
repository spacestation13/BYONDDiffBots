use std::io::Write;

pub struct DefaultLogger;

impl log::Log for DefaultLogger {
    fn enabled(&self, _metadata: &log::Metadata) -> bool {
        #[cfg(debug_assertions)]
        return true;
        #[cfg(not(debug_assertions))]
        return _metadata.level() <= log::LevelFilter::Info;
    }
    fn log(&self, record: &log::Record) {
        if self.enabled(record.metadata()) {
            println!("{} - {}", record.level(), record.args());
        }
    }
    fn flush(&self) {}
}

//this is obviously more expensive than the above
pub struct FileLogger {
    filename: &'static str,
}

impl log::Log for FileLogger {
    fn enabled(&self, _metadata: &log::Metadata) -> bool {
        #[cfg(debug_assertions)]
        return true;
        #[cfg(not(debug_assertions))]
        return _metadata.level() <= log::LevelFilter::Info;
    }
    fn log(&self, record: &log::Record) {
        if self.enabled(record.metadata()) {
            if let Err(err) = std::fs::OpenOptions::new()
                .append(true)
                .create(true)
                .open(self.filename)
                .and_then(|mut file| writeln!(file, "{} - {}", record.level(), record.args()))
            {
                println!("{}", err)
            };
        }
    }
    fn flush(&self) {}
}
