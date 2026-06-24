//! SQLite persistence, config loading, and XDG path resolution.

mod paths;

pub use paths::AppPaths;

use lorelm_core::{
    AppState, Conversation, ConversationId, ConversationState, DocumentPanelState, GenerationState,
    Message, MessageRole, MessageStatus, ModeDefinition, ModeId, ModelId, ModelPanelState, UiState,
    Workspace, WorkspaceId, WorkspaceState,
};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use time::{OffsetDateTime, format_description::well_known::Rfc3339};

/// Storage-layer error.
#[derive(Debug, thiserror::Error)]
pub enum StorageError {
    /// Directory resolution failed.
    #[error("could not resolve user directories")]
    DirectoryResolution,
    /// Filesystem operation failed.
    #[error("filesystem error: {0}")]
    Io(#[from] std::io::Error),
    /// Database operation failed.
    #[error("database error: {0}")]
    Database(#[from] rusqlite::Error),
    /// TOML serialization or parsing failed.
    #[error("toml error: {0}")]
    Toml(#[from] toml::de::Error),
    /// Timestamp formatting failed.
    #[error("time formatting error: {0}")]
    Time(#[from] time::error::Format),
    /// UUID parsing failed.
    #[error("uuid parsing error: {0}")]
    Uuid(#[from] uuid::Error),
}

/// Convenient result type for storage operations.
pub type Result<T> = std::result::Result<T, StorageError>;

/// Global application configuration loaded from `config.toml`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AppConfig {
    /// Configured model paths.
    pub paths: PathConfig,
    /// UI options.
    pub ui: UiConfig,
    /// Global inference runtime settings.
    pub inference: InferenceConfig,
    /// Retrieval defaults.
    pub retrieval: RetrievalConfig,
    /// Indexing defaults.
    pub indexing: IndexingConfig,
    /// Chunking defaults.
    pub chunking: ChunkingConfig,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            paths: PathConfig {
                model_dirs: vec!["~/.models".to_owned()],
                model_path: None,
            },
            ui: UiConfig {
                theme: "default".to_owned(),
                show_sources_panel: true,
            },
            inference: InferenceConfig {
                defaults: InferenceDefaults {
                    context_size: 8192,
                    threads: 8,
                    batch_size: 512,
                    ubatch_size: 128,
                    use_mmap: true,
                    use_mlock: false,
                },
            },
            retrieval: RetrievalConfig {
                strategy: "hybrid".to_owned(),
                vector_top_k: 24,
                fts_top_k: 24,
                final_top_k: 8,
                max_chunks_per_document: 3,
                vector_weight: 0.7,
                fts_weight: 0.3,
                diversity_bonus: 0.05,
                heading_match_bonus: 0.1,
            },
            indexing: IndexingConfig {
                pause_during_generation: true,
                max_parallel_embedding_batches: 1,
            },
            chunking: ChunkingConfig {
                target_tokens: 500,
                max_tokens: 800,
                overlap_tokens: 80,
                min_tokens: 80,
            },
        }
    }
}

/// Path configuration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PathConfig {
    /// Directories scanned for models.
    pub model_dirs: Vec<String>,
    /// Optional Phase 1 model path used before model scanning exists.
    #[serde(default)]
    pub model_path: Option<String>,
}

/// UI configuration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UiConfig {
    /// Theme name.
    pub theme: String,
    /// Whether to show source panels.
    pub show_sources_panel: bool,
}

/// Inference configuration wrapper.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InferenceConfig {
    /// Global runtime defaults.
    pub defaults: InferenceDefaults,
}

/// Global inference runtime defaults.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InferenceDefaults {
    /// Context size.
    pub context_size: usize,
    /// Worker threads.
    pub threads: usize,
    /// Batch size.
    pub batch_size: usize,
    /// Micro-batch size.
    pub ubatch_size: usize,
    /// Whether to use mmap.
    pub use_mmap: bool,
    /// Whether to use mlock.
    pub use_mlock: bool,
}

/// Retrieval configuration.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RetrievalConfig {
    /// Retrieval strategy.
    pub strategy: String,
    /// Vector candidate count.
    pub vector_top_k: usize,
    /// FTS candidate count.
    pub fts_top_k: usize,
    /// Final result count.
    pub final_top_k: usize,
    /// Per-document cap.
    pub max_chunks_per_document: usize,
    /// Vector score weight.
    pub vector_weight: f32,
    /// FTS score weight.
    pub fts_weight: f32,
    /// Diversity bonus.
    pub diversity_bonus: f32,
    /// Heading-match bonus.
    pub heading_match_bonus: f32,
}

/// Indexing configuration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IndexingConfig {
    /// Whether indexing pauses during generation.
    pub pause_during_generation: bool,
    /// Maximum embedding batches.
    pub max_parallel_embedding_batches: usize,
}

/// Chunking configuration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChunkingConfig {
    /// Target chunk token estimate.
    pub target_tokens: usize,
    /// Maximum chunk token estimate.
    pub max_tokens: usize,
    /// Overlap token estimate.
    pub overlap_tokens: usize,
    /// Minimum chunk token estimate.
    pub min_tokens: usize,
}

/// IDs created during database bootstrap.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Bootstrap {
    /// Active workspace ID.
    pub workspace_id: WorkspaceId,
    /// Active conversation ID.
    pub conversation_id: ConversationId,
}

/// SQLite-backed storage handle.
pub struct Storage {
    paths: AppPaths,
    connection: Connection,
}

impl Storage {
    /// Opens storage and applies migrations.
    pub fn open(paths: &AppPaths) -> Result<Self> {
        paths.ensure_all()?;
        let connection = Connection::open(paths.database_file())?;
        connection.pragma_update(None, "foreign_keys", "ON")?;
        let storage = Self {
            paths: paths.clone(),
            connection,
        };
        storage.migrate()?;
        Ok(storage)
    }

    /// Loads config from disk, creating defaults when no file exists.
    pub fn load_config(&self) -> Result<AppConfig> {
        if !self.paths.config_file().exists() {
            let config = AppConfig::default();
            let encoded = toml::to_string_pretty(&config).map_err(|error| {
                StorageError::Io(std::io::Error::new(std::io::ErrorKind::InvalidData, error))
            })?;
            std::fs::write(self.paths.config_file(), encoded)?;
            return Ok(config);
        }

        let content = std::fs::read_to_string(self.paths.config_file())?;
        Ok(toml::from_str(&content)?)
    }

    /// Ensures the default Phase 0 workspace and conversation exist.
    pub fn bootstrap(&self) -> Result<Bootstrap> {
        let now = now()?;
        let workspace_id = self
            .connection
            .query_row(
                "SELECT id FROM workspaces ORDER BY created_at LIMIT 1",
                [],
                |row| row.get::<_, String>(0),
            )
            .optional()?
            .map(|id| id.parse())
            .transpose()?
            .unwrap_or_else(WorkspaceId::new);

        self.connection.execute(
            "INSERT OR IGNORE INTO workspaces (id, name, root_path, created_at, updated_at)
             VALUES (?1, 'default', NULL, ?2, ?2)",
            params![workspace_id.to_string(), now],
        )?;

        let conversation_id = self
            .connection
            .query_row(
                "SELECT id FROM conversations WHERE workspace_id = ?1 ORDER BY created_at LIMIT 1",
                params![workspace_id.to_string()],
                |row| row.get::<_, String>(0),
            )
            .optional()?
            .map(|id| id.parse())
            .transpose()?
            .unwrap_or_else(ConversationId::new);

        self.connection.execute(
            "INSERT OR IGNORE INTO conversations
                (id, workspace_id, parent_conversation_id, title, active_mode_id, created_at, updated_at)
             VALUES (?1, ?2, NULL, 'main', 'freeform', ?3, ?3)",
            params![conversation_id.to_string(), workspace_id.to_string(), now],
        )?;

        Ok(Bootstrap {
            workspace_id,
            conversation_id,
        })
    }

    /// Loads the state required to start the TUI.
    pub fn load_app_state(
        &self,
        workspace_id: &WorkspaceId,
        conversation_id: &ConversationId,
        _config: AppConfig,
        available_ram_bytes: u64,
    ) -> Result<AppState> {
        let workspace = self.load_workspace(workspace_id)?;
        let conversation = self.load_conversation(conversation_id)?;
        let messages = self.load_messages(conversation_id)?;

        Ok(AppState {
            workspace: WorkspaceState {
                active_workspace: Some(workspace),
            },
            conversation: ConversationState {
                active_conversation: Some(conversation),
                messages,
                streaming_response: None,
                scroll_offset: 0,
            },
            documents: DocumentPanelState::default(),
            models: ModelPanelState::default(),
            generation: GenerationState::Idle,
            active_mode: ModeDefinition::freeform(),
            ui: UiState::default(),
            available_ram_bytes,
        })
    }

    /// Returns the next message sequence for a conversation.
    pub fn next_message_sequence(&self, conversation_id: &ConversationId) -> Result<i64> {
        let next = self.connection.query_row(
            "SELECT COALESCE(MAX(sequence), -1) + 1 FROM messages WHERE conversation_id = ?1",
            params![conversation_id.to_string()],
            |row| row.get(0),
        )?;
        Ok(next)
    }

    /// Inserts a message.
    pub fn insert_message(&self, message: &Message) -> Result<()> {
        let created_at = if message.created_at.is_empty() {
            now()?
        } else {
            message.created_at.clone()
        };
        self.connection.execute(
            "INSERT INTO messages
                (id, conversation_id, sequence, role, content, model_id, mode_id, status, token_count, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                message.id.to_string(),
                message.conversation_id.to_string(),
                message.sequence,
                message.role.as_str(),
                message.content,
                message.model_id.map(|id| id.to_string()),
                message.mode_id.as_ref().map(|id| id.as_str().to_owned()),
                message.status.as_str(),
                message.token_count,
                created_at,
            ],
        )?;
        self.connection.execute(
            "UPDATE conversations SET updated_at = ?1 WHERE id = ?2",
            params![created_at, message.conversation_id.to_string()],
        )?;
        Ok(())
    }

    fn migrate(&self) -> Result<()> {
        self.connection.execute_batch(SCHEMA)?;
        Ok(())
    }

    fn load_workspace(&self, workspace_id: &WorkspaceId) -> Result<Workspace> {
        self.connection
            .query_row(
                "SELECT id, name, root_path, created_at, updated_at FROM workspaces WHERE id = ?1",
                params![workspace_id.to_string()],
                |row| {
                    let id: String = row.get(0)?;
                    Ok(Workspace {
                        id: id.parse().map_err(|error| {
                            rusqlite::Error::FromSqlConversionFailure(
                                0,
                                rusqlite::types::Type::Text,
                                Box::new(error),
                            )
                        })?,
                        name: row.get(1)?,
                        root_path: row.get(2)?,
                        created_at: row.get(3)?,
                        updated_at: row.get(4)?,
                    })
                },
            )
            .map_err(StorageError::from)
    }

    fn load_conversation(&self, conversation_id: &ConversationId) -> Result<Conversation> {
        self.connection
            .query_row(
                "SELECT id, workspace_id, parent_conversation_id, title, active_mode_id, created_at, updated_at
                 FROM conversations WHERE id = ?1",
                params![conversation_id.to_string()],
                |row| {
                    let id: String = row.get(0)?;
                    let workspace_id: String = row.get(1)?;
                    let parent_conversation_id: Option<String> = row.get(2)?;
                    let active_mode_id: String = row.get(4)?;
                    Ok(Conversation {
                        id: parse_id(id, 0)?,
                        workspace_id: parse_id(workspace_id, 1)?,
                        parent_conversation_id: parent_conversation_id
                            .map(|id| parse_id(id, 2))
                            .transpose()?,
                        title: row.get(3)?,
                        active_mode_id: ModeId::named(active_mode_id),
                        created_at: row.get(5)?,
                        updated_at: row.get(6)?,
                    })
                },
            )
            .map_err(StorageError::from)
    }

    fn load_messages(&self, conversation_id: &ConversationId) -> Result<Vec<Message>> {
        let mut statement = self.connection.prepare(
            "SELECT id, conversation_id, sequence, role, content, model_id, mode_id, status, token_count, created_at
             FROM messages
             WHERE conversation_id = ?1 AND status != 'cancelled'
             ORDER BY sequence",
        )?;
        let rows = statement.query_map(params![conversation_id.to_string()], |row| {
            let id: String = row.get(0)?;
            let conversation_id: String = row.get(1)?;
            let role: String = row.get(3)?;
            let status: String = row.get(7)?;
            Ok(Message {
                id: parse_id(id, 0)?,
                conversation_id: parse_id(conversation_id, 1)?,
                sequence: row.get(2)?,
                role: parse_role(&role, 3)?,
                content: row.get(4)?,
                model_id: row
                    .get::<_, Option<String>>(5)?
                    .map(parse_model_id)
                    .transpose()?,
                mode_id: row.get::<_, Option<String>>(6)?.map(ModeId::named),
                status: parse_status(&status, 7)?,
                token_count: row.get(8)?,
                created_at: row.get(9)?,
            })
        })?;

        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(StorageError::from)
    }
}

fn parse_id<T>(value: String, index: usize) -> rusqlite::Result<T>
where
    T: std::str::FromStr,
    T::Err: std::error::Error + Send + Sync + 'static,
{
    value.parse().map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(
            index,
            rusqlite::types::Type::Text,
            Box::new(error),
        )
    })
}

fn parse_model_id(value: String) -> rusqlite::Result<ModelId> {
    parse_id(value, 5)
}

fn parse_role(value: &str, index: usize) -> rusqlite::Result<MessageRole> {
    match value {
        "system" => Ok(MessageRole::System),
        "user" => Ok(MessageRole::User),
        "assistant" => Ok(MessageRole::Assistant),
        _ => Err(rusqlite::Error::FromSqlConversionFailure(
            index,
            rusqlite::types::Type::Text,
            format!("unknown message role: {value}").into(),
        )),
    }
}

fn parse_status(value: &str, index: usize) -> rusqlite::Result<MessageStatus> {
    match value {
        "complete" => Ok(MessageStatus::Complete),
        "cancelled" => Ok(MessageStatus::Cancelled),
        "error" => Ok(MessageStatus::Error),
        _ => Err(rusqlite::Error::FromSqlConversionFailure(
            index,
            rusqlite::types::Type::Text,
            format!("unknown message status: {value}").into(),
        )),
    }
}

fn now() -> Result<String> {
    Ok(OffsetDateTime::now_utc().format(&Rfc3339)?)
}

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS workspaces (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    root_path TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS documents (
    id TEXT PRIMARY KEY,
    workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    source_kind TEXT NOT NULL,
    original_path TEXT,
    display_name TEXT NOT NULL,
    content_hash TEXT NOT NULL,
    mime TEXT,
    size_bytes INTEGER,
    imported_at TEXT NOT NULL,
    status TEXT NOT NULL,
    error_message TEXT
);

CREATE TABLE IF NOT EXISTS document_texts (
    document_id TEXT PRIMARY KEY REFERENCES documents(id) ON DELETE CASCADE,
    extracted_text TEXT NOT NULL,
    structure_json TEXT,
    snapshot_path TEXT
);

CREATE TABLE IF NOT EXISTS chunks (
    id TEXT PRIMARY KEY,
    document_id TEXT NOT NULL REFERENCES documents(id) ON DELETE CASCADE,
    workspace_id TEXT NOT NULL REFERENCES workspaces(id),
    sequence INTEGER NOT NULL,
    heading_path TEXT,
    source_label TEXT NOT NULL,
    start_byte INTEGER,
    end_byte INTEGER,
    page_number INTEGER,
    token_estimate INTEGER,
    text TEXT NOT NULL,
    content_hash TEXT NOT NULL
);

CREATE VIRTUAL TABLE IF NOT EXISTS chunk_fts USING fts5(
    chunk_id UNINDEXED,
    text,
    heading_path,
    source_label
);

CREATE TABLE IF NOT EXISTS embedding_models (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    provider TEXT NOT NULL,
    dimension INTEGER NOT NULL,
    config_json TEXT,
    created_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS chunk_embeddings (
    chunk_id TEXT NOT NULL REFERENCES chunks(id) ON DELETE CASCADE,
    embedding_model_id TEXT NOT NULL REFERENCES embedding_models(id),
    dimension INTEGER NOT NULL,
    created_at TEXT NOT NULL,
    PRIMARY KEY (chunk_id, embedding_model_id)
);

CREATE TABLE IF NOT EXISTS chunk_vec_map (
    vec_rowid INTEGER PRIMARY KEY,
    chunk_id TEXT NOT NULL REFERENCES chunks(id) ON DELETE CASCADE,
    embedding_model_id TEXT NOT NULL REFERENCES embedding_models(id)
);

CREATE TABLE IF NOT EXISTS conversations (
    id TEXT PRIMARY KEY,
    workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    parent_conversation_id TEXT REFERENCES conversations(id),
    title TEXT,
    active_mode_id TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS messages (
    id TEXT PRIMARY KEY,
    conversation_id TEXT NOT NULL REFERENCES conversations(id) ON DELETE CASCADE,
    sequence INTEGER NOT NULL,
    role TEXT NOT NULL,
    content TEXT NOT NULL,
    model_id TEXT,
    mode_id TEXT,
    status TEXT NOT NULL,
    token_count INTEGER,
    created_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS message_sources (
    id TEXT PRIMARY KEY,
    message_id TEXT NOT NULL REFERENCES messages(id) ON DELETE CASCADE,
    chunk_id TEXT NOT NULL REFERENCES chunks(id),
    rank INTEGER NOT NULL,
    score REAL,
    source_label TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS models (
    id TEXT PRIMARY KEY,
    display_name TEXT NOT NULL,
    provider TEXT NOT NULL,
    local_path TEXT,
    repo_id TEXT,
    filename TEXT,
    size_bytes INTEGER,
    quantization TEXT,
    architecture TEXT,
    context_train INTEGER,
    chat_template TEXT,
    discovered_from TEXT NOT NULL,
    installed_at TEXT,
    last_used_at TEXT
);

CREATE TABLE IF NOT EXISTS model_downloads (
    id TEXT PRIMARY KEY,
    model_id TEXT,
    repo_id TEXT NOT NULL,
    filename TEXT NOT NULL,
    destination_path TEXT NOT NULL,
    status TEXT NOT NULL,
    bytes_downloaded INTEGER NOT NULL DEFAULT 0,
    total_bytes INTEGER,
    error TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);
"#;

#[cfg(test)]
mod tests {
    use super::*;
    use lorelm_core::{MessageRole, MessageStatus};

    #[test]
    fn bootstrap_persists_stub_messages_and_reloads_history() {
        let root =
            std::env::temp_dir().join(format!("lorelm-storage-test-{}", uuid::Uuid::new_v4()));
        let paths = AppPaths::from_roots(
            root.join("config"),
            root.join("data"),
            root.join("cache"),
            root.join("state"),
        );

        let storage = Storage::open(&paths).expect("storage opens");
        let config = storage.load_config().expect("config loads");
        let bootstrap = storage.bootstrap().expect("bootstrap succeeds");
        let message = Message::new(
            bootstrap.conversation_id,
            0,
            MessageRole::User,
            "hello".to_owned(),
            MessageStatus::Complete,
        );
        storage.insert_message(&message).expect("message inserts");
        drop(storage);

        let storage = Storage::open(&paths).expect("storage reopens");
        let state = storage
            .load_app_state(
                &bootstrap.workspace_id,
                &bootstrap.conversation_id,
                config,
                0,
            )
            .expect("state reloads");

        assert_eq!(state.conversation.messages.len(), 1);
        assert_eq!(state.conversation.messages[0].content, "hello");
        assert_eq!(state.conversation.messages[0].role, MessageRole::User);

        std::fs::remove_dir_all(root).expect("test directory is removed");
    }
}
