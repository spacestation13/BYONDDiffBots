use tracing::Level;
use tracing_subscriber::fmt::writer::MakeWriterExt;
use tracing_subscriber::prelude::*;

fn get_log_level(log_level: &str) -> Level {
    match log_level {
        "trace" => Level::TRACE,
        "debug" => Level::DEBUG,
        "info" => Level::INFO,
        "warn" => Level::WARN,
        "error" => Level::ERROR,
        _ => Level::ERROR,
    }
}

pub fn init_logger(
    log_level: &str,
    grafana_layer: Option<tracing_loki::Layer>,
) -> eyre::Result<()> {
    // let builder = tracing_subscriber::fmt()
    //     .with_level(true)
    //     .with_line_number(true)
    //     .with_max_level(get_log_level(log_level));
    let stdout_layer = tracing_subscriber::fmt::layer()
        .with_level(true)
        .with_line_number(true)
        .with_writer(std::io::stdout.with_max_level(get_log_level(log_level)));
    if let Some(layer) = grafana_layer {
        tracing_subscriber::registry()
            .with(layer)
            .with(stdout_layer)
            .init();
    } else {
        tracing_subscriber::registry().with(stdout_layer).init();
    }

    std::panic::set_hook(Box::new(tracing_panic::panic_hook));
    Ok(())
}
