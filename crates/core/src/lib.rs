//! Shared domain types, commands, events, and application state.

mod ids;

pub use ids::*;

use serde::{Deserialize, Serialize};

/// Application-level error data that can be rendered by the TUI.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppError {
    /// Human-readable error message.
    pub message: String,
}

impl AppError {
    /// Creates a new user-facing application error.
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

/// Commands emitted by the TUI and handled by the app coordinator.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Command {
    /// Submit a user prompt for the active conversation.
    SubmitPrompt {
        /// Conversation that receives the exchange.
        conversation_id: ConversationId,
        /// Prompt content entered by the user.
        content: String,
    },
    /// Cancel an active generation if one exists.
    CancelGeneration,
    /// Scroll the transcript by the signed line delta.
    ScrollTranscript { delta: isize },
    /// Move focus to a specific panel.
    SetFocus(Panel),
    /// Open an application screen.
    OpenScreen(Screen),
    /// Dismiss the visible error.
    DismissError,
    /// Shut down the application.
    Quit,
}

/// Events emitted by workers or the coordinator and applied by the TUI reducer.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum AppEvent {
    /// Periodic render tick.
    Tick,
    /// A message was persisted and should be appended to the transcript.
    MessageAppended(Message),
    /// A document import task changed status.
    DocumentImportProgress(ImportTask),
    /// A model download task changed status.
    ModelDownloadProgress(DownloadTask),
    /// A model became active.
    ModelLoaded(ModelId),
    /// Retrieval completed.
    RetrievalFinished(RetrievalResult),
    /// A generated token delta arrived.
    TokenDelta(String),
    /// Generation completed with a summary.
    GenerationFinished(GenerationSummary),
    /// Generation was cancelled.
    GenerationCancelled,
    /// An error should be surfaced to the user.
    Error(AppError),
}

/// Complete state required to render the application.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AppState {
    /// Workspace state.
    pub workspace: WorkspaceState,
    /// Conversation state.
    pub conversation: ConversationState,
    /// Document panel state.
    pub documents: DocumentPanelState,
    /// Model panel state.
    pub models: ModelPanelState,
    /// Current generation state.
    pub generation: GenerationState,
    /// Active mode definition.
    pub active_mode: ModeDefinition,
    /// Transient UI state.
    pub ui: UiState,
    /// Available RAM estimate in bytes.
    pub available_ram_bytes: u64,
}

/// Active workspace data.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorkspaceState {
    /// Selected workspace, if one exists.
    pub active_workspace: Option<Workspace>,
}

/// Active conversation data.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConversationState {
    /// Selected conversation, if one exists.
    pub active_conversation: Option<Conversation>,
    /// Visible messages. Cancelled messages are excluded by storage.
    pub messages: Vec<Message>,
    /// Transcript scroll offset.
    pub scroll_offset: usize,
}

/// Document sidebar state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct DocumentPanelState {
    /// Known document summaries.
    pub documents: Vec<DocumentSummary>,
    /// Active import tasks.
    pub import_tasks: Vec<ImportTask>,
}

/// Model panel state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ModelPanelState {
    /// Active model, if one is loaded.
    pub active_model: Option<LocalModel>,
    /// Installed local models.
    pub installed_models: Vec<LocalModel>,
    /// Active download tasks.
    pub download_tasks: Vec<DownloadTask>,
}

/// Transient UI state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UiState {
    /// Active screen.
    pub active_screen: Screen,
    /// Focused panel.
    pub focused_panel: Panel,
    /// Modal stack.
    pub modal_stack: Vec<Modal>,
    /// Last error shown in the status bar.
    pub error: Option<AppError>,
}

impl Default for UiState {
    fn default() -> Self {
        Self {
            active_screen: Screen::Chat,
            focused_panel: Panel::Prompt,
            modal_stack: Vec::new(),
            error: None,
        }
    }
}

/// Top-level application screens.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Screen {
    /// Primary chat screen.
    Chat,
    /// File import screen.
    FileImport,
    /// Model library screen.
    ModelLibrary,
    /// Mode and prompt editor.
    ModeEditor,
    /// Settings screen.
    Settings,
}

/// Focusable UI panels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Panel {
    /// Document and session sidebar.
    Sidebar,
    /// Conversation transcript.
    Transcript,
    /// Prompt editor.
    Prompt,
    /// Status bar.
    Status,
}

/// Placeholder modal identifiers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Modal {
    /// Generic error modal.
    Error(String),
}

/// Generation state machine.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum GenerationState {
    /// No generation is active.
    Idle,
    /// Prompt assembly or context budgeting is running.
    Preparing,
    /// Retrieval is running.
    Retrieving,
    /// Tokens are streaming.
    Generating,
    /// Cancellation was requested.
    Cancelling,
    /// Generation failed.
    Failed(String),
}

/// A workspace/project.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Workspace {
    /// Stable workspace ID.
    pub id: WorkspaceId,
    /// Display name.
    pub name: String,
    /// Optional root path as stored text.
    pub root_path: Option<String>,
    /// Creation timestamp.
    pub created_at: String,
    /// Update timestamp.
    pub updated_at: String,
}

/// A linear conversation within a workspace.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Conversation {
    /// Stable conversation ID.
    pub id: ConversationId,
    /// Owning workspace ID.
    pub workspace_id: WorkspaceId,
    /// Optional parent conversation.
    pub parent_conversation_id: Option<ConversationId>,
    /// Optional title.
    pub title: Option<String>,
    /// Active mode ID.
    pub active_mode_id: ModeId,
    /// Creation timestamp.
    pub created_at: String,
    /// Update timestamp.
    pub updated_at: String,
}

/// Chat message.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Message {
    /// Stable message ID.
    pub id: MessageId,
    /// Owning conversation ID.
    pub conversation_id: ConversationId,
    /// Monotonic sequence within a conversation.
    pub sequence: i64,
    /// Message role.
    pub role: MessageRole,
    /// Message body.
    pub content: String,
    /// Optional model ID.
    pub model_id: Option<ModelId>,
    /// Optional mode ID.
    pub mode_id: Option<ModeId>,
    /// Persistence status.
    pub status: MessageStatus,
    /// Optional token count.
    pub token_count: Option<i64>,
    /// Creation timestamp.
    pub created_at: String,
}

impl Message {
    /// Creates a new message with a generated ID and empty timestamp.
    pub fn new(
        conversation_id: ConversationId,
        sequence: i64,
        role: MessageRole,
        content: String,
        status: MessageStatus,
    ) -> Self {
        Self {
            id: MessageId::new(),
            conversation_id,
            sequence,
            role,
            content,
            model_id: None,
            mode_id: None,
            status,
            token_count: None,
            created_at: String::new(),
        }
    }
}

/// Message role.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MessageRole {
    /// System message.
    System,
    /// User message.
    User,
    /// Assistant message.
    Assistant,
}

impl MessageRole {
    /// Returns the database representation.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::System => "system",
            Self::User => "user",
            Self::Assistant => "assistant",
        }
    }
}

/// Message persistence status.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MessageStatus {
    /// Completed message.
    Complete,
    /// Cancelled message.
    Cancelled,
    /// Error message.
    Error,
}

impl MessageStatus {
    /// Returns the database representation.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Complete => "complete",
            Self::Cancelled => "cancelled",
            Self::Error => "error",
        }
    }
}

/// Document summary rendered in the sidebar.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DocumentSummary {
    /// Stable document ID.
    pub id: DocumentId,
    /// Display name.
    pub display_name: String,
    /// Import/index status.
    pub status: String,
    /// Optional error text.
    pub error_message: Option<String>,
}

/// Local model metadata.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LocalModel {
    /// Stable model ID.
    pub id: ModelId,
    /// Display name.
    pub display_name: String,
    /// Provider identifier.
    pub provider: ModelProvider,
    /// Optional local path.
    pub local_path: Option<String>,
}

/// Supported model providers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ModelProvider {
    /// Local GGUF file.
    LocalGguf,
    /// Hugging Face GGUF artifact.
    HfGguf,
    /// Remote OpenAI-compatible model.
    RemoteOpenAiCompatible,
}

/// Import task status.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImportTask {
    /// A human-readable task label.
    pub label: String,
    /// A human-readable task status.
    pub status: String,
}

/// Download task status.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DownloadTask {
    /// A human-readable task label.
    pub label: String,
    /// A human-readable task status.
    pub status: String,
}

/// Retrieval result placeholder.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RetrievalResult {
    /// Number of retrieved chunks.
    pub chunk_count: usize,
}

/// Generation summary placeholder.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GenerationSummary {
    /// Model used for generation.
    pub model_id: Option<ModelId>,
    /// Number of generated tokens.
    pub generated_tokens: usize,
    /// Context token estimate.
    pub context_tokens_used: usize,
    /// Throughput estimate.
    pub tokens_per_second: f32,
    /// Stop reason.
    pub stop_reason: StopReason,
    /// Duration in milliseconds.
    pub duration_ms: u64,
}

/// Generation stop reason.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum StopReason {
    /// End-of-sequence token.
    EndOfSequence,
    /// Maximum token count reached.
    MaxTokens,
    /// User cancellation.
    Cancelled,
    /// Error string.
    Error(String),
}

/// Parsed document produced by document ingestion.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParsedDocument {
    /// Optional title.
    pub title: Option<String>,
    /// Source metadata.
    pub source: DocumentSource,
    /// Structured blocks.
    pub blocks: Vec<DocumentBlock>,
    /// Full extracted text.
    pub raw_text: String,
}

/// Document source metadata.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DocumentSource {
    /// Display label.
    pub label: String,
    /// Optional original path.
    pub path: Option<String>,
}

/// A parsed document block.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DocumentBlock {
    /// Block kind.
    pub kind: DocumentBlockKind,
    /// Block text.
    pub text: String,
    /// Heading path surrounding this block.
    pub heading_path: Vec<String>,
    /// Source byte span.
    pub source_span: Option<SourceSpan>,
}

/// Parsed document block kind.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum DocumentBlockKind {
    /// Heading with a level.
    Heading { level: u8 },
    /// Paragraph block.
    Paragraph,
    /// Code block.
    CodeBlock { language: Option<String> },
    /// List item.
    ListItem,
    /// Block quote.
    BlockQuote,
    /// Table.
    Table,
    /// Page break.
    PageBreak,
}

/// Byte span in a source document.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceSpan {
    /// Start byte offset.
    pub start_byte: usize,
    /// End byte offset.
    pub end_byte: usize,
}

/// Mode definition and defaults.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModeDefinition {
    /// Stable mode ID.
    pub id: ModeId,
    /// Display name.
    pub name: String,
    /// Description.
    pub description: String,
    /// System prompt.
    pub system_prompt: String,
    /// Retrieval policy.
    pub retrieval_policy: RetrievalPolicy,
    /// Whether answers require sources.
    pub require_sources: bool,
    /// Generation defaults.
    pub generation: GenerationConfig,
    /// Conversation context policy.
    pub context: ContextPolicy,
}

impl ModeDefinition {
    /// Returns the built-in freeform mode used by Phase 0.
    pub fn freeform() -> Self {
        Self {
            id: ModeId::named("freeform"),
            name: "Freeform".to_owned(),
            description: "No retrieval, no constraints.".to_owned(),
            system_prompt: "You are a local writing assistant.".to_owned(),
            retrieval_policy: RetrievalPolicy::None,
            require_sources: false,
            generation: GenerationConfig {
                temperature: 0.7,
                top_p: 0.9,
                repeat_penalty: 1.1,
                max_tokens: 512,
            },
            context: ContextPolicy {
                include_recent_messages: true,
                max_recent_messages: 8,
            },
        }
    }
}

/// Retrieval policy for a mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RetrievalPolicy {
    /// No retrieval.
    None,
    /// Full-text search.
    Fts,
    /// Vector search.
    Vector,
    /// Hybrid vector and full-text search.
    Hybrid,
}

/// Generation defaults.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GenerationConfig {
    /// Sampling temperature.
    pub temperature: f32,
    /// Nucleus sampling cutoff.
    pub top_p: f32,
    /// Repeat penalty.
    pub repeat_penalty: f32,
    /// Generation cap and output reservation.
    pub max_tokens: usize,
}

/// Conversation context policy.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContextPolicy {
    /// Whether to include recent messages.
    pub include_recent_messages: bool,
    /// Maximum recent messages to include.
    pub max_recent_messages: usize,
}
