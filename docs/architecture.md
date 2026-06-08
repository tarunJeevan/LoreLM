# LoreLM — Architecture

## 1. Project Structure

LoreLM is a **modular monolith** structured as a Cargo workspace with internal crates. No microservices. Crate-level separation provides clear boundaries without deployment complexity, appropriate for a single-binary local TUI application.

### Cargo Workspace Layout

```
LoreLM/
  Cargo.toml
  app/                  # binary crate — coordinator, wires all service crates
  crates/
    app-tui/            # ratatui frontend and event loop; depends only on core
    core/               # shared domain types, commands, app state — zero I/O
    storage/            # SQLite, config files, migrations — pure persistence
    doc-ingest/         # txt, md, later pdf/epub import
    retrieval/          # chunking, FTS, vector index, hybrid retriever, context packing
    embeddings/         # pure compute: text in, Vec<f32> out; no DB knowledge
    inference/          # InferenceBackend trait, prompt assembly, context budget
    llama-backend/      # llama-cpp-2 implementation; all llama-cpp-2 types contained here
    model-manager/      # local model registry, downloads, metadata
```

### Crate Dependency Graph

```
app (binary)
  ├── app-tui           (depends only on core)
  ├── core              (no I/O, no DB; serde + uuid only)
  ├── storage           (depends on core; pure persistence)
  ├── doc-ingest        (depends on core)
  ├── retrieval         (depends on core, embeddings, storage)
  ├── embeddings        (pure compute; minimal deps)
  ├── inference         (depends on core)
  ├── llama-backend     (depends on inference/core; owns all llama-cpp-2 types)
  └── model-manager     (depends on core, storage)
```

### System Architecture Diagram

```
┌────────────────────────────────────────────────────────────────────┐
│                            app-tui                                 │
│  ratatui views: chat, doc browser, model library, mode editor,     │
│  model settings, status bar                                        │
│  Depends only on core. Sends Commands, receives AppEvents.         │
└───────────────┬────────────────────────────────────────────────────┘
                │ Sender<Command> / Receiver<AppEvent>
                ▼
┌────────────────────────────────────────────────────────────────────┐
│                         app (coordinator)                          │
│  Wires service crates · starts background workers                  │
│  Dispatches Commands to services · forwards AppEvents to app-tui   │
└──┬──────────────┬──────────────┬──────────────┬────────────────────┘
   │              │              │              │
   ▼              ▼              ▼              ▼
┌────────┐ ┌──────────┐ ┌────────────┐ ┌──────────────┐
│storage │ │doc-ingest│ │model-mgr   │ │  inference   │
│SQLite  │ │txt/md    │ │scan/dl/reg │ │  worker thd  │
│TOML    │ │normalize │ │settings    │ │  WorkerCmd   │
└────┬───┘ └────┬─────┘ └────────────┘ └──────┬───────┘
     │          │                             │
     │          ▼                             │ llama-backend
     │   ┌─────────────────────────────┐      │ (owns all
     └──►│         retrieval           │      │  llama-cpp-2
         │  chunker · FTS · vec index  │      │  types)
         │  hybrid retriever           │      │
         │  context packer             │      │
         │      ▼                      │      │
         │  embeddings (pure compute)  │      │
         └─────────────────────────────┘      │
                    │ retrieved context       │
                    └─────────────────────────┘
                              ▼
                    core (domain types only)
```

---

## 2. Tech Stack

| Layer | Technology | Notes |
|---|---|---|
| Language | Rust (stable) | Cargo workspace, edition 2021 |
| TUI | `ratatui` + `crossterm` | Immediate-mode terminal UI |
| Async runtime | `tokio` | Downloads, file import, indexing, event channels |
| Inference | `llama-cpp-2` | Wrapped behind `InferenceBackend`; isolated to `llama-backend` |
| Embeddings | `fastembed` + `ort` | MiniLM/BGE-small class; ONNX runtime |
| Database | SQLite via `rusqlite` (bundled) | App data, chunks, messages, embeddings metadata |
| Vector search | `sqlite-vec` (`vec0` virtual table) | Wrapped behind `VectorIndex` trait |
| Full-text search | SQLite FTS5 (built-in) | Non-content virtual table with explicit `chunk_id` |
| Config/presets | TOML via `toml` + `serde` | Human-editable; XDG-compliant paths |
| DB migrations | `refinery` or `barrel` | Versioned schema migrations |
| Serialization | `serde` + `serde_json` | Domain types and SQLite metadata blobs |
| IDs | `uuid` | Stable IDs for all domain entities |
| Timestamps | `time` | Preferred over `chrono` |
| XDG paths | `directories` | Config, data, cache, state directories |
| Error handling | `thiserror` (internal) + `anyhow` (app-level) | See §5 |
| Logging | `tracing` + `tracing-subscriber` + `tracing-appender` | Structured; rolling log files |
| HTTP / downloads | `reqwest` + `hf-hub` | Model downloads; HF Hub client |
| Checksum | `sha2` | Model file hash validation |
| Markdown parsing | `pulldown-cmark` | CommonMark + extensions |
| Encoding | `encoding_rs` | Non-UTF-8 text normalization |
| File traversal | `walkdir` + `ignore` | Folder import with `.gitignore` awareness |

### Dependency Tables by Crate Group

**TUI and Input**

| Crate | Use |
|---|---|
| `ratatui` | Core immediate-mode terminal UI |
| `crossterm` | Terminal input, events, backend |
| `tui-textarea` or custom | Multiline prompt editing |
| `unicode-width` | Correct wide-char layout |

**Async, Events, Concurrency**

| Crate | Use |
|---|---|
| `tokio` | Async runtime |
| `async-trait` | Async backend traits |
| `crossbeam-channel` or `tokio::sync::mpsc` | Streaming tokens and progress events |
| `parking_lot` | Lightweight locks for local state |

**Storage and Retrieval**

| Crate | Use |
|---|---|
| `rusqlite` (bundled) | SQLite access |
| `refinery` or `barrel` | DB migrations |
| `sqlite-vec` | Vector storage/search; wrap behind `VectorIndex` trait |
| `zerocopy` | Efficient embedding byte passing for `sqlite-vec` |

**Future Remote/Server (Phase 8)**

| Crate | Use |
|---|---|
| `async-openai` or custom `reqwest` | OpenAI-compatible backend |
| `axum` | Optional local API server |
| `tower` | Middleware for server mode |

---

## 3. Data Storage Model

### Access Model

LoreLM is a local-only, single-user application. There is no authentication, no multi-user access control, and no network-facing server in v1. Data access is governed entirely by OS-level file permissions on the SQLite database, model files, and config files.

### XDG Directory Layout

```
$XDG_CONFIG_HOME/LoreLM/
  config.toml
  modes/
    document_qa.toml
    summarizer.toml
    editor_rewriter.toml
    brainstorming.toml
  model-settings/<model-id>.toml

$XDG_DATA_HOME/LoreLM/
  app.db
  models/<repo-or-model-name>/model.gguf · metadata.toml
  document-snapshots/<document-id>.txt

$XDG_CACHE_HOME/LoreLM/downloads/
  embedding-models/
  temp/

$XDG_STATE_HOME/LoreLM/
  app.log
```

### Config Precedence Stack

Settings are resolved at generation time by merging layers lowest-to-highest priority:

| Priority | Source | Scope |
|---|---|---|
| 1 (lowest) | `global config.toml [inference.defaults]` | All models and modes |
| 2 | Mode TOML `[defaults]` | This mode, all models |
| 3 | Per-model TOML `[defaults.<mode_id>]` | This mode, this model |
| 4 (highest) | Workspace TOML *(reserved for v2; not implemented)* | This project only |

### Settings Ownership

Each setting belongs to exactly one config layer. Placing a setting in the wrong layer is a configuration error.

| Setting | Global config | Mode TOML | Per-model TOML |
|---|---|---|---|
| `context_size` | default | ✗ never | override |
| `threads`, `batch_size`, `ubatch_size` | default | ✗ never | override |
| `use_mmap`, `use_mlock` | default | ✗ never | override |
| `temperature`, `top_p`, `repeat_penalty` | ✗ never | default | per-mode override |
| `max_tokens` | ✗ never | default | per-mode override |
| `max_recent_messages` | ✗ never | defined here only | ✗ never |
| `retrieval_policy` | ✗ never | defined here only | ✗ never |
| `require_sources` | ✗ never | defined here only | ✗ never |

### SQLite Schema

```sql
CREATE TABLE workspaces (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    root_path TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE TABLE documents (
    id TEXT PRIMARY KEY,
    workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    source_kind TEXT NOT NULL,        -- file | paste
    original_path TEXT,
    display_name TEXT NOT NULL,
    content_hash TEXT NOT NULL,
    mime TEXT,
    size_bytes INTEGER,
    imported_at TEXT NOT NULL,
    status TEXT NOT NULL,             -- pending | parsed | chunked | indexed | embedded | ready | failed
    error_message TEXT                -- populated on status = 'failed'
);

CREATE TABLE document_texts (
    document_id TEXT PRIMARY KEY REFERENCES documents(id) ON DELETE CASCADE,
    extracted_text TEXT NOT NULL,
    structure_json TEXT,
    snapshot_path TEXT
);

CREATE TABLE chunks (
    id TEXT PRIMARY KEY,
    document_id TEXT NOT NULL REFERENCES documents(id) ON DELETE CASCADE,
    workspace_id TEXT NOT NULL REFERENCES workspaces(id),
    sequence INTEGER NOT NULL,
    heading_path TEXT,
    source_label TEXT NOT NULL,
    start_byte INTEGER,
    end_byte INTEGER,
    page_number INTEGER,              -- optional; populated for PDF sources
    token_estimate INTEGER,
    text TEXT NOT NULL,
    content_hash TEXT NOT NULL
);

-- Non-content FTS5 table. Stores its own copy of indexed text.
-- chunk_id stored explicitly for join-back; no content= or content_rowid=
-- to avoid TEXT PK / integer rowid mismatch.
CREATE VIRTUAL TABLE chunk_fts USING fts5(
    chunk_id UNINDEXED,
    text,
    heading_path,
    source_label
);

CREATE TABLE embedding_models (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    provider TEXT NOT NULL,           -- fastembed | llama.cpp | remote
    dimension INTEGER NOT NULL,
    config_json TEXT,
    created_at TEXT NOT NULL
);

-- INVARIANT: chunk_embeddings and chunk_vec_map must be written in a
-- single SQLite transaction. Partial failure must roll back both.
-- Every chunk_vec_map row must have a corresponding chunk_embeddings row.
CREATE TABLE chunk_embeddings (
    chunk_id TEXT NOT NULL REFERENCES chunks(id) ON DELETE CASCADE,
    embedding_model_id TEXT NOT NULL REFERENCES embedding_models(id),
    dimension INTEGER NOT NULL,
    created_at TEXT NOT NULL,
    PRIMARY KEY (chunk_id, embedding_model_id)
);

-- sqlite-vec virtual table (dimension example: 384)
CREATE VIRTUAL TABLE chunk_vec USING vec0(embedding float[384]);

CREATE TABLE chunk_vec_map (
    vec_rowid INTEGER PRIMARY KEY,
    chunk_id TEXT NOT NULL REFERENCES chunks(id) ON DELETE CASCADE,
    embedding_model_id TEXT NOT NULL REFERENCES embedding_models(id)
);

CREATE TABLE conversations (
    id TEXT PRIMARY KEY,
    workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    parent_conversation_id TEXT REFERENCES conversations(id),
    title TEXT,
    active_mode_id TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE TABLE messages (
    id TEXT PRIMARY KEY,
    conversation_id TEXT NOT NULL REFERENCES conversations(id) ON DELETE CASCADE,
    sequence INTEGER NOT NULL,
    role TEXT NOT NULL,               -- system | user | assistant
    content TEXT NOT NULL,
    model_id TEXT,
    mode_id TEXT,
    status TEXT NOT NULL,             -- complete | cancelled | error
    token_count INTEGER,
    created_at TEXT NOT NULL
);

CREATE TABLE message_sources (
    id TEXT PRIMARY KEY,
    message_id TEXT NOT NULL REFERENCES messages(id) ON DELETE CASCADE,
    chunk_id TEXT NOT NULL REFERENCES chunks(id),
    rank INTEGER NOT NULL,
    score REAL,
    source_label TEXT NOT NULL
);

CREATE TABLE models (
    id TEXT PRIMARY KEY,
    display_name TEXT NOT NULL,
    provider TEXT NOT NULL,           -- LocalGguf | HfGguf | RemoteOpenAiCompatible
    local_path TEXT,
    repo_id TEXT,
    filename TEXT,
    size_bytes INTEGER,
    quantization TEXT,
    architecture TEXT,
    context_train INTEGER,
    chat_template TEXT,
    discovered_from TEXT NOT NULL,    -- directory_scan | curated_catalog | manual_path
    installed_at TEXT,
    last_used_at TEXT
);

CREATE TABLE model_downloads (
    id TEXT PRIMARY KEY,
    model_id TEXT,
    repo_id TEXT NOT NULL,
    filename TEXT NOT NULL,
    destination_path TEXT NOT NULL,
    status TEXT NOT NULL,             -- queued | downloading | complete | failed |
                                      -- cancelled | terminal_failed | terminal_cancelled
    bytes_downloaded INTEGER NOT NULL DEFAULT 0,
    total_bytes INTEGER,
    error TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);
```

---

## 4. Module Responsibilities and Core Traits

### `core`

Zero I/O. Zero DB. No external deps beyond `serde` and `uuid`. All other crates depend on `core`.

**Domain IDs:** `WorkspaceId`, `DocumentId`, `ConversationId`, `MessageId`, `ModelId`, `ModeId`, `ChunkId`.

**Document parsing types (produced by `doc-ingest`, consumed by `retrieval`):**

```rust
struct ParsedDocument {
    title: Option<String>,
    source: DocumentSource,
    blocks: Vec<DocumentBlock>,
    raw_text: String,
}

struct DocumentBlock {
    kind: DocumentBlockKind,
    text: String,
    heading_path: Vec<String>,
    source_span: Option<SourceSpan>,
}

enum DocumentBlockKind {
    Heading { level: u8 },
    Paragraph,
    CodeBlock { language: Option<String> },
    ListItem,
    BlockQuote,
    Table,
    PageBreak,
}
```

**Mode definitions:**

```rust
struct ModeDefinition {
    id: ModeId,
    name: String,
    description: String,
    system_prompt: String,
    retrieval_policy: RetrievalPolicy,
    require_sources: bool,
    generation: GenerationConfig,
    context: ContextPolicy,
}

enum RetrievalPolicy {
    None,
    Fts,
    Vector,
    Hybrid,
}

struct GenerationConfig {
    temperature: f32,
    top_p: f32,
    repeat_penalty: f32,
    max_tokens: usize,  // caps generation AND is reserved by ContextBudget
}

struct ContextPolicy {
    include_recent_messages: bool,
    max_recent_messages: usize,
}
```

**Events and commands:**

```rust
enum AppEvent {
    KeyPressed(KeyEvent),
    Tick,
    DocumentImportProgress(ImportTask),
    ModelDownloadProgress(DownloadTask),
    ModelLoaded(ModelId),
    RetrievalFinished(RetrievalResult),
    TokenDelta(String),
    GenerationFinished(GenerationSummary),
    GenerationCancelled,
    Error(AppError),
}
```

**AppState and sub-state structs:**

```rust
struct AppState {
    workspace: WorkspaceState,
    conversation: ConversationState,
    documents: DocumentPanelState,
    models: ModelPanelState,
    generation: GenerationState,
    active_mode: ModeDefinition,
    ui: UiState,
    available_ram_bytes: u64,         // populated at startup by ResourcePlanner
}

struct WorkspaceState {
    active_workspace: Option<Workspace>,
}

struct ConversationState {
    active_conversation: Option<Conversation>,
    messages: Vec<Message>,           // excludes status = 'cancelled'
    scroll_offset: usize,
}

struct DocumentPanelState {
    documents: Vec<DocumentSummary>,  // id, display_name, status, error_message
    import_tasks: Vec<ImportTask>,
}

struct ModelPanelState {
    active_model: Option<LocalModel>,
    installed_models: Vec<LocalModel>,
    download_tasks: Vec<DownloadTask>,
}

struct UiState {
    active_screen: Screen,
    focused_panel: Panel,
    modal_stack: Vec<Modal>,
}

enum Screen {
    Chat,
    FileImport,
    ModelLibrary,
    ModeEditor,
    Settings,
}

enum GenerationState {
    Idle,
    Preparing,       // ContextBudget computation, prompt assembly
    Retrieving,      // RAG query in flight (retrieval modes only)
    Generating,      // tokens streaming from inference worker
    Cancelling,      // cancel signalled; awaiting GenerationCancelled event
    Failed(String),  // error message for status bar / modal
}
```

### `app` (binary — coordinator)

Owns references to all service crates. Spawns the persistent inference worker thread and all background task workers. Holds `Sender<WorkerCommand>` for the inference worker. Receives `Command`s from `app-tui` and dispatches to the appropriate service. Forwards `AppEvent`s back to `app-tui`. The only crate permitted to depend on everything.

### `app-tui`

Terminal setup/teardown, input handling, main event loop, ratatui rendering, mapping input to `Command`s. Manages transient UI state (focus, cursor, scroll, modal stack, prompt editor buffer). Depends only on `core` — no direct access to `storage`, `retrieval`, `inference`, or any service crate.

### `storage`

Opens and migrates the SQLite DB. Stores and retrieves all domain entities. Loads and saves TOML config. Resolves XDG directories. All multi-table writes use explicit transactions. Pure persistence — no business logic, no retrieval logic.

### `doc-ingest`

Imports pasted text and `.txt`/`.md` files (later `.pdf`, `.epub`). Normalizes encodings via `encoding_rs`. Parses Markdown via `pulldown-cmark`. Produces `ParsedDocument` (defined in `core`). Does not chunk, does not write to the DB.

### `retrieval`

Orchestrates the full indexing and query pipeline. Chunks `ParsedDocument`s, writes chunks to storage, calls `embeddings` for chunk and query embedding, writes vectors to storage, executes hybrid FTS+vector queries, merges and ranks results, packs context within budget. Depends on `core`, `embeddings`, and `storage`.

```rust
trait Retriever {
    async fn retrieve(&self, request: RetrievalRequest) -> Result<RetrievalResult>;
}

trait Chunker {
    fn chunk(&self, document: &ParsedDocument, config: ChunkingConfig) -> Vec<DocumentChunk>;
}

trait ContextPacker {
    fn pack(&self, request: ContextPackRequest) -> Result<PackedContext>;
}

struct HybridRetrievalConfig {
    vector_top_k: usize,            // default: 24
    fts_top_k: usize,               // default: 24
    final_top_k: usize,             // default: 8 (runtime cap applied by ContextBudget)
    max_chunks_per_document: usize, // default: 3
    vector_weight: f32,             // default: 0.7
    fts_weight: f32,                // default: 0.3
    diversity_bonus: f32,           // default: 0.05
    heading_match_bonus: f32,       // default: 0.1
}
```

### `embeddings`

Pure computation crate. Loads an embedding model via `fastembed` (ONNX via `ort`). Embeds arbitrary text; returns `Vec<f32>`. No knowledge of chunks, documents, or the database.

### `inference`

Backend-neutral API. Prompt assembly. Context budget calculation. Stream/cancel protocol.

```rust
#[async_trait::async_trait]
trait InferenceBackend: Send {
    async fn load_model(&mut self, spec: ModelSpec, config: RuntimeModelConfig) -> Result<()>;
    async fn unload_model(&mut self) -> Result<()>;
    async fn generate_stream(
        &mut self,
        request: GenerateRequest,
        sink: TokenSink,
        cancel: CancellationToken,
    ) -> Result<GenerationSummary>;
    fn current_model(&self) -> Option<ModelId>;
    fn estimate_tokens(&self, text: &str) -> Result<usize>;
}

enum StopReason {
    EndOfSequence,
    MaxTokens,
    Cancelled,
    Error(String),
}

struct GenerationSummary {
    model_id: ModelId,
    generated_tokens: usize,
    context_tokens_used: usize,
    tokens_per_second: f32,
    stop_reason: StopReason,
    duration_ms: u64,
}

enum WorkerCommand {
    LoadModel(ModelSpec, RuntimeModelConfig),
    Generate(GenerateRequest, TokenSink, CancellationToken),
    UnloadModel,
    Shutdown,
}
```

### `llama-backend`

Owned by the persistent inference worker thread. Owns all `llama-cpp-2` types — nothing leaks to other crates. Translates `ModelSpec` and `RuntimeModelConfig` into llama.cpp params. Streams tokens via channel. Supports cancellation.

### `model-manager`

Scans model directories. Maintains local model registry in `storage`. Tracks and executes downloads. Validates files via hash check. Stores and loads per-model TOML settings.

Scanned directories:
```
$XDG_DATA_HOME/LoreLM/models/
$HOME/.models/
custom paths from config.toml
```

---

## 5. Invariants

These are rules the codebase must never violate. Most are enforced at compile time by crate boundaries; the rest are stated explicitly here and must be upheld in code review.

### Compile-time enforced

**I-1: `core` has zero I/O and zero DB access.**
`core` depends only on `serde` and `uuid`. Any I/O import in `core` is a build-time dependency error.

**I-2: `app-tui` depends only on `core`.**
`app-tui` may not import `storage`, `retrieval`, `inference`, `llama-backend`, `embeddings`, `doc-ingest`, or `model-manager`. All data flows through `AppEvent`; all actions flow through `Command`.

**I-3: All `llama-cpp-2` types are contained in `llama-backend`.**
No `llama-cpp-2` type may appear in `inference`, `core`, `app-tui`, or any other crate.

**I-4: `embeddings` has no knowledge of the database.**
`embeddings` may not import `storage`, `rusqlite`, or any DB crate. It receives text, returns `Vec<f32>`.

**I-5: `storage` contains no business logic.**
`storage` persists and retrieves data. It does not make retrieval decisions, does not call `embeddings`, and does not assemble prompts.

### Runtime enforced (must be upheld in implementation)

**I-6: `chunk_embeddings` and `chunk_vec_map` are always written in a single transaction.**
Inserting into one without the other in the same transaction is a bug. Partial failure must roll back both.

**I-7: `chunk_fts` entries are manually deleted before their corresponding chunk row is dropped.**
`chunk_fts` is a virtual table with no FK relationship to `chunks`. On document re-import or deletion, `chunk_fts` rows must be deleted by `chunk_id` before the `chunks` row is removed.

**I-8: Cancelled messages are never included in model context.**
History packing in `ContextBudget` must filter out all `messages` rows where `status = 'cancelled'` before assembling the prompt.

**I-9: The inference worker thread is the sole owner of the `llama-backend` instance.**
No other thread may hold a reference to or interact with `llama-backend` directly. All interaction goes through `WorkerCommand` via the channel.

**I-10: Background workers never mutate `AppState` directly.**
All state updates flow through `AppEvent` sent to `app-tui`. Workers produce events; the reducer in `app-tui` applies them.

**I-11: `GenerationState` transitions must follow the defined state machine.**
Invalid transitions (e.g. `Idle → Generating`, skipping `Preparing`) are bugs. Enforce via the state machine defined in `core`.

```
Idle        → Preparing
Preparing   → Retrieving    (if RetrievalPolicy != None)
Preparing   → Generating    (if RetrievalPolicy == None)
Retrieving  → Generating
Generating  → Idle          (on GenerationFinished)
Generating  → Cancelling    (on user Ctrl+C)
Cancelling  → Idle          (on GenerationCancelled event)
Any state   → Failed        (on Error event)
Failed      → Idle          (on user dismiss or next prompt submission)
```

**I-12: TOML settings must not be placed in the wrong config layer.**
Runtime settings (`context_size`, `threads`, etc.) may never appear in mode TOML. Generation settings (`temperature`, `max_tokens`, etc.) may never appear in global config. See the Settings Ownership table in §3.
