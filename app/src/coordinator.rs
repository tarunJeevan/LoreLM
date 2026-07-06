use std::{path::PathBuf, sync::mpsc as std_mpsc, thread, time::Duration};

use anyhow::Context;
use inference::{
    CancellationToken, GenerateRequest, GenerationEvent, InferenceBackend, ModelSpec, WorkerCommand,
};
use lorelm_core::{
    AppConfig, AppError, AppEvent, Command, ConversationId, Message, MessageId, MessageRole,
    MessageStatus, ModeDefinition, ModelId, StopReason,
};
use model_manager::ResourcePlanner;
use storage::Storage;
use tokio::sync::mpsc;

pub struct Coordinator {
    storage: Storage,
    command_rx: mpsc::Receiver<Command>,
    event_tx: mpsc::Sender<AppEvent>,
    worker_tx: std_mpsc::Sender<WorkerCommand>,
    generation_tx: std_mpsc::Sender<GenerationEvent>,
    generation_rx: std_mpsc::Receiver<GenerationEvent>,
    worker_handle: Option<thread::JoinHandle<()>>,
    active_generation: Option<ActiveGeneration>,
    active_cancel: Option<CancellationToken>,
    config: AppConfig,
    available_ram_bytes: u64,
}

impl Coordinator {
    pub fn new(
        storage: Storage,
        command_rx: mpsc::Receiver<Command>,
        event_tx: mpsc::Sender<AppEvent>,
        config: AppConfig,
        available_ram_bytes: u64,
    ) -> Self {
        let (worker_tx, worker_rx) = std_mpsc::channel();
        let (generation_tx, generation_rx) = std_mpsc::channel();
        let worker_handle = thread::spawn(move || run_inference_worker(worker_rx));

        let coordinator = Self {
            storage,
            command_rx,
            event_tx,
            worker_tx,
            generation_tx,
            generation_rx,
            worker_handle: Some(worker_handle),
            active_generation: None,
            active_cancel: None,
            config,
            available_ram_bytes,
        };
        coordinator.load_configured_model();
        coordinator
    }

    pub fn run(mut self) -> anyhow::Result<()> {
        let mut command_rx_closed = false;
        loop {
            self.drain_generation_events()?;

            match self.command_rx.try_recv() {
                Ok(Command::Quit) => break,
                Ok(command) => self.handle_command(command)?,
                Err(mpsc::error::TryRecvError::Empty) => {}
                Err(mpsc::error::TryRecvError::Disconnected) => command_rx_closed = true,
            }

            if command_rx_closed {
                break;
            }

            thread::sleep(Duration::from_millis(20));
        }

        let _ = self.worker_tx.send(WorkerCommand::Shutdown);
        if let Some(handle) = self.worker_handle.take() {
            handle
                .join()
                .map_err(|_| anyhow::anyhow!("inference worker thread panicked"))?;
        }

        Ok(())
    }

    fn handle_command(&mut self, command: Command) -> anyhow::Result<()> {
        match command {
            Command::SubmitPrompt {
                conversation_id,
                content,
            } => self.start_generation(conversation_id, content)?,
            Command::CancelGeneration => {
                if let Some(cancel) = &self.active_cancel {
                    cancel.cancel();
                }
            }
            Command::DismissError
            | Command::ScrollTranscript { .. }
            | Command::SetFocus(_)
            | Command::OpenScreen(_)
            | Command::Quit => {}
        }

        Ok(())
    }

    fn start_generation(
        &mut self,
        conversation_id: ConversationId,
        content: String,
    ) -> anyhow::Result<()> {
        if self.active_generation.is_some() {
            let _ = self
                .event_tx
                .blocking_send(AppEvent::Error(AppError::new("generation already active")));
            return Ok(());
        }

        let next_sequence = self
            .storage
            .next_message_sequence(&conversation_id)
            .context("failed to load next message sequence")?;

        let mut user_message = Message::new(
            conversation_id,
            next_sequence,
            MessageRole::User,
            content.clone(),
            MessageStatus::Complete,
        );
        user_message.mode_id = Some(ModeDefinition::freeform().id);
        self.storage
            .insert_message(&user_message)
            .context("failed to persist user message")?;
        let user_message_id = user_message.id;
        let _ = self
            .event_tx
            .blocking_send(AppEvent::MessageAppended(user_message));

        let cancel = CancellationToken::new();
        self.active_generation = Some(ActiveGeneration {
            conversation_id,
            user_message_id,
            assistant_sequence: next_sequence + 1,
            mode_id: ModeDefinition::freeform().id,
            content: String::new(),
            model_id: None,
        });
        self.active_cancel = Some(cancel.clone());

        let request = GenerateRequest {
            conversation_id,
            prompt: content,
            mode_id: ModeDefinition::freeform().id,
            generation: ModeDefinition::freeform().generation,
        };

        self.worker_tx
            .send(WorkerCommand::Generate {
                request,
                sink: self.generation_tx.clone(),
                cancel,
            })
            .context("failed to send generation command")?;

        Ok(())
    }

    fn drain_generation_events(&mut self) -> anyhow::Result<()> {
        while let Ok(event) = self.generation_rx.try_recv() {
            self.handle_generation_event(event)?;
        }
        Ok(())
    }

    fn handle_generation_event(&mut self, event: GenerationEvent) -> anyhow::Result<()> {
        match event {
            GenerationEvent::ModelLoaded(model_id) => {
                let _ = self.event_tx.blocking_send(AppEvent::ModelLoaded(model_id));
            }
            GenerationEvent::TokenDelta(delta) => {
                if let Some(active_generation) = &mut self.active_generation {
                    active_generation.content.push_str(&delta);
                }
                let _ = self.event_tx.blocking_send(AppEvent::TokenDelta(delta));
            }
            GenerationEvent::Finished(summary) => {
                if summary.stop_reason == StopReason::Cancelled {
                    self.cancel_active_generation(summary.model_id)?;
                    let _ = self.event_tx.blocking_send(AppEvent::GenerationCancelled);
                    return Ok(());
                }

                if let Some(active_generation) = self.active_generation.take() {
                    let mut assistant_message = Message::new(
                        active_generation.conversation_id,
                        active_generation.assistant_sequence,
                        MessageRole::Assistant,
                        active_generation.content,
                        MessageStatus::Complete,
                    );
                    assistant_message.model_id = summary.model_id;
                    assistant_message.mode_id = Some(active_generation.mode_id);
                    assistant_message.token_count = Some(summary.generated_tokens as i64);
                    self.storage
                        .insert_message(&assistant_message)
                        .context("failed to persist assistant message")?;
                    let _ = self
                        .event_tx
                        .blocking_send(AppEvent::MessageAppended(assistant_message));
                }
                self.active_cancel = None;
                let _ = self
                    .event_tx
                    .blocking_send(AppEvent::GenerationFinished(summary));
            }
            GenerationEvent::Cancelled => {
                self.cancel_active_generation(None)?;
                self.active_cancel = None;
                let _ = self.event_tx.blocking_send(AppEvent::GenerationCancelled);
            }
            GenerationEvent::Failed(message) => {
                self.active_generation = None;
                self.active_cancel = None;
                let _ = self
                    .event_tx
                    .blocking_send(AppEvent::Error(AppError::new(message)));
            }
        }

        Ok(())
    }

    fn load_configured_model(&self) {
        let Some(path) = &self.config.paths.model_path else {
            return;
        };

        let path = expand_home(path);
        let display_name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("configured model")
            .to_owned();
        let model_spec = ModelSpec {
            id: ModelId::new(),
            display_name,
            size_bytes: path.metadata().ok().map(|metadata| metadata.len()),
            path,
        };

        let planner = ResourcePlanner::new();
        let mut runtime_config = planner.plan(&model_spec, self.available_ram_bytes);
        let defaults = &self.config.inference.defaults;
        runtime_config.context_size = defaults.context_size;
        runtime_config.threads = defaults.threads;
        runtime_config.batch_size = defaults.batch_size;
        runtime_config.ubatch_size = defaults.ubatch_size;
        runtime_config.use_mmap = defaults.use_mmap;
        runtime_config.use_mlock = defaults.use_mlock;
        let _ = self.worker_tx.send(WorkerCommand::LoadModel {
            spec: model_spec,
            config: runtime_config,
            sink: self.generation_tx.clone(),
        });
    }

    fn cancel_active_generation(&mut self, model_id: Option<ModelId>) -> anyhow::Result<()> {
        if let Some(active_generation) = self.active_generation.take() {
            self.storage
                .update_message_status(&active_generation.user_message_id, MessageStatus::Cancelled)
                .context("failed to mark user message cancelled")?;

            if !active_generation.content.is_empty() {
                let mut assistant_message = Message::new(
                    active_generation.conversation_id,
                    active_generation.assistant_sequence,
                    MessageRole::Assistant,
                    active_generation.content,
                    MessageStatus::Cancelled,
                );
                assistant_message.model_id = model_id.or(active_generation.model_id);
                assistant_message.mode_id = Some(active_generation.mode_id);
                self.storage
                    .insert_message(&assistant_message)
                    .context("failed to persist cancelled assistant message")?;
            }
        }
        self.active_cancel = None;
        Ok(())
    }
}

impl Drop for Coordinator {
    fn drop(&mut self) {
        let _ = self.worker_tx.send(WorkerCommand::Shutdown);
    }
}

#[derive(Debug)]
struct ActiveGeneration {
    conversation_id: ConversationId,
    user_message_id: MessageId,
    assistant_sequence: i64,
    mode_id: lorelm_core::ModeId,
    content: String,
    model_id: Option<ModelId>,
}

fn run_inference_worker(rx: std_mpsc::Receiver<WorkerCommand>) {
    let mut backend = llama_backend::LlamaBackend::new();
    while let Ok(command) = rx.recv() {
        match command {
            WorkerCommand::LoadModel { spec, config, sink } => {
                let model_id = spec.id;
                match backend.load_model(spec, config) {
                    Ok(()) => {
                        let _ = sink.send(GenerationEvent::ModelLoaded(model_id));
                    }
                    Err(error) => {
                        let _ = sink.send(GenerationEvent::Failed(error.to_string()));
                    }
                }
            }
            WorkerCommand::Generate {
                request,
                sink,
                cancel,
            } => match backend.generate_stream(request, sink.clone(), cancel) {
                Ok(summary) => {
                    let terminal_event = if summary.stop_reason == StopReason::Cancelled {
                        GenerationEvent::Cancelled
                    } else {
                        GenerationEvent::Finished(summary)
                    };
                    let _ = sink.send(terminal_event);
                }
                Err(error) => {
                    let _ = sink.send(GenerationEvent::Failed(error.to_string()));
                }
            },
            WorkerCommand::UnloadModel { sink } => {
                if let Err(error) = backend.unload_model() {
                    let _ = sink.send(GenerationEvent::Failed(error.to_string()));
                }
            }
            WorkerCommand::Shutdown => break,
        }
    }
}

fn expand_home(path: &str) -> PathBuf {
    if let Some(stripped) = path.strip_prefix("~/")
        && let Some(home) = std::env::var_os("HOME")
    {
        return PathBuf::from(home).join(stripped);
    }
    PathBuf::from(path)
}
