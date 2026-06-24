//! Backend-neutral inference API, worker commands, and generation protocol.

use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
};

use lorelm_core::{ConversationId, GenerationConfig, ModeId, ModelId};

pub use lorelm_core::{GenerationSummary, StopReason};

/// Convenient result type for inference operations.
pub type Result<T> = std::result::Result<T, InferenceError>;

/// Inference error.
#[derive(Debug, thiserror::Error)]
pub enum InferenceError {
    /// No model has been loaded.
    #[error("no model is loaded")]
    NoModelLoaded,
    /// Model loading failed.
    #[error("failed to load model: {0}")]
    ModelLoad(String),
    /// Text generation failed.
    #[error("generation failed: {0}")]
    Generation(String),
}

/// Backend-neutral description of a model to load.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelSpec {
    /// Stable model ID.
    pub id: ModelId,
    /// User-facing model name.
    pub display_name: String,
    /// Local GGUF path.
    pub path: PathBuf,
    /// File size in bytes, when known.
    pub size_bytes: Option<u64>,
}

/// Runtime settings used to load and run a model.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeModelConfig {
    /// Context window size.
    pub context_size: usize,
    /// CPU worker threads.
    pub threads: usize,
    /// Prompt batch size.
    pub batch_size: usize,
    /// Prompt micro-batch size.
    pub ubatch_size: usize,
    /// Whether to memory-map model weights.
    pub use_mmap: bool,
    /// Whether to lock model memory.
    pub use_mlock: bool,
}

/// A single generation request.
#[derive(Debug, Clone, PartialEq)]
pub struct GenerateRequest {
    /// Conversation receiving the generated assistant message.
    pub conversation_id: ConversationId,
    /// Current user prompt or assembled prompt text.
    pub prompt: String,
    /// Active mode ID.
    pub mode_id: ModeId,
    /// Generation defaults for the active mode.
    pub generation: GenerationConfig,
}

/// Events emitted by the inference worker during model loading and generation.
#[derive(Debug, Clone, PartialEq)]
pub enum GenerationEvent {
    /// A model loaded successfully.
    ModelLoaded(ModelId),
    /// A token delta should be appended to the streaming response.
    TokenDelta(String),
    /// Generation finished successfully.
    Finished(GenerationSummary),
    /// The active generation was cancelled.
    Cancelled,
    /// The worker hit an unrecoverable operation error.
    Failed(String),
}

/// Channel used by a backend to emit generation events.
pub type TokenSink = mpsc::Sender<GenerationEvent>;

/// Commands sent to the persistent inference worker.
#[derive(Debug)]
pub enum WorkerCommand {
    /// Load a model into the backend.
    LoadModel {
        /// Model to load.
        spec: ModelSpec,
        /// Runtime settings.
        config: RuntimeModelConfig,
        /// Event sink for load completion or failure.
        sink: TokenSink,
    },
    /// Generate text from the currently loaded model.
    Generate {
        /// Generation request.
        request: GenerateRequest,
        /// Event sink for token and terminal generation events.
        sink: TokenSink,
        /// Cancellation handle checked by the backend.
        cancel: CancellationToken,
    },
    /// Unload the active model.
    UnloadModel {
        /// Event sink for unload failures.
        sink: TokenSink,
    },
    /// Stop the worker thread.
    Shutdown,
}

/// Shared cancellation flag for the active generation.
#[derive(Debug, Clone, Default)]
pub struct CancellationToken {
    cancelled: Arc<AtomicBool>,
}

impl CancellationToken {
    /// Creates a new unset cancellation token.
    pub fn new() -> Self {
        Self::default()
    }

    /// Requests cancellation.
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
    }

    /// Returns whether cancellation has been requested.
    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::SeqCst)
    }
}

/// Backend-neutral interface implemented by concrete inference engines.
pub trait InferenceBackend: Send {
    /// Load a model using the supplied runtime settings.
    fn load_model(&mut self, spec: ModelSpec, config: RuntimeModelConfig) -> Result<()>;

    /// Unload the active model, if one is loaded.
    fn unload_model(&mut self) -> Result<()>;

    /// Generate text and stream token deltas through `sink`.
    fn generate_stream(
        &mut self,
        request: GenerateRequest,
        sink: TokenSink,
        cancel: CancellationToken,
    ) -> Result<GenerationSummary>;

    /// Returns the active model ID, if loaded.
    fn current_model(&self) -> Option<ModelId>;

    /// Estimates token count for text.
    fn estimate_tokens(&self, text: &str) -> Result<usize>;
}
