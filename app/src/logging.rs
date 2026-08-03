use anyhow::Context;
use storage::AppPaths;
use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::{EnvFilter, fmt};

pub fn init(paths: &AppPaths) -> anyhow::Result<WorkerGuard> {
    std::fs::create_dir_all(paths.state_dir()).context("failed to create state directory")?;
    let file_appender = tracing_appender::rolling::never(paths.state_dir(), "app.log");
    let (writer, guard) = tracing_appender::non_blocking(file_appender);
    let filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("lorelm=info"));

    fmt().with_env_filter(filter).with_writer(writer).init();

    Ok(guard)
}
