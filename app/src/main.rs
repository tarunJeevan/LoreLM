mod coordinator;
mod logging;

use anyhow::Context;
use coordinator::Coordinator;
use lorelm_core::Command;
use tokio::sync::mpsc;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let paths = storage::AppPaths::resolve().context("failed to resolve LoreLM paths")?;
    let _logging_guard = logging::init(&paths).context("failed to initialize logging")?;

    let storage = storage::Storage::open(&paths).context("failed to open storage")?;
    let config = storage
        .load_config()
        .context("failed to load config.toml")?;
    tracing::info!(config_path = %paths.config_file().display(), "loaded configuration");

    let bootstrap = storage
        .bootstrap()
        .context("failed to bootstrap database")?;
    let initial_state = storage
        .load_app_state(&bootstrap.workspace_id, &bootstrap.conversation_id, config)
        .context("failed to load application state")?;

    let (command_tx, command_rx) = mpsc::channel::<Command>(64);
    let (event_tx, event_rx) = mpsc::channel(64);

    let coordinator = Coordinator::new(storage, command_rx, event_tx);
    let coordinator_handle = std::thread::spawn(move || coordinator.run());

    let tui_result = app_tui::run(initial_state, command_tx, event_rx).await;
    if let Err(error) = coordinator_handle
        .join()
        .map_err(|_| anyhow::anyhow!("coordinator thread panicked"))?
    {
        tracing::error!(%error, "coordinator exited with error");
    }

    tui_result
}
