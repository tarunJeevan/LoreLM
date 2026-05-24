# LoreLM - Architecture Document

## 1. Architectural Stance

LoreLM is a **modular monolith** structured as a Cargo workspace with internal crates. No microservices. This provides crate-level separation without deployment complexity and is appropriate for a single-binary local TUI application.

### Cargo Workspace Layout

```
LoreLM/
  Cargo.toml
  app/             # binary crate — coordinator, wires all service crates
  crates/
    app-tui/       # ratatui frontend and event loop; depends only on core
    core/          # shared domain types, commands, app state — zero I/O
    storage/       # SQLite, config files, migrations — pure persistence
    doc-ingest/    # txt, md, later pdf/epub import
    retrieval/     # chunking, FTS, vector index, hybrid retriever, context packing
    embeddings/    # pure compute: text in, Vec<f32> out; no DB knowledge
    inference/     # InferenceBackend trait, prompt assembly, context budget
    llama-backend/ # llama-cpp-2 implementation; all llama-cpp-2 types contained here
    model-manager/ # local model registry, downloads, metadata
```

### Crate Dependency Rules

These rules are enforced at the compiler level by crate boundaries. Violations are build errors, not guidelines.

- **`core`** - zero I/O, zero DB access. External dependencies limited to `serde` and `uuid`. All other crates depend on `core`; `core` depends on nothing internal.
- **`storage`** - all SQLite access, all TOML file I/O, all XDG path resolution. Depends on `core`. No business logic.
- **`app-tui`** - depends only on `core`. All inputs arrive as `AppEvent`s; all outputs are `Command`s. No direct access to `storage`, `retrieval`, `inference`, or any service crate.
- **`app` (binary)** - owns the coordinator. Holds references to all service crates, starts background workers, wires `app-tui` via a `(Sender<Command>, Receiver<AppEvent>)` channel pair. The only crate permitted to depend on everything.
- **`retrieval`** - depends on `core`, `embeddings`, and `storage`. Orchestrates the full pipeline: chunk → embed → store vector → query.
- **`embeddings`** - pure computation. Depends on `fastembed`/`ort`. No knowledge of chunks, documents, or the database.
- **`llama-backend`** - all `llama-cpp-2` types are contained here. Nothing leaks into `inference`, `core`, or `app-tui`.

### Config Precedence Stack

Settings are resolved at generation time by merging layers lowest-to-highest priority:

1. **`global config.toml [inference.defaults]`** - baseline for all models and modes
2. **Mode TOML `[defaults]`** - canonical generation behavior per mode
3. **Per-model TOML `[defaults.<mode_id>]`** - model-specific override for a given mode
4. **Workspace TOML** - project-scoped overrides *(reserved for v2; not implemented in v1)*

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

### Resulting Dependency Graph

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

---

## 2. System Architecture Diagram

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

## 3. Key Architectural Decisions

### Decision 1: Workspace-first data model

A workspace owns documents, extracted text, chunks, embeddings, conversations, prompt history, generated messages, and exportable transcripts. Models are not owners of conversation history — a model is one possible generator used inside a workspace. This allows multiple models to access the same project conversation history.

### Decision 2: Mode-scoped retrieval

RAG is a first-class concern but not forced onto every interaction:

| Mode            | Retrieval behavior                                                        |
|-----------------|---------------------------------------------------------------------------|
| Document Q&A    | Hybrid retrieval (vector + FTS) over workspace documents                  |
| Summarizer      | Direct full-context for small docs; map-reduce for docs exceeding budget  |
| Editor/Rewriter | Direct selected/pasted text; no retrieval                                 |
| Brainstorming   | No retrieval unless user explicitly attaches context                      |
| Freeform        | No retrieval                                                              |

**Summarizer map-reduce (v1, minimal):** If `document_token_estimate > context_size - max_tokens - system_prompt_tokens`, split via the RAG chunker, summarize each chunk sequentially (map pass: "Summarize this excerpt faithfully and concisely"), then synthesize partial summaries (reduce pass: "Synthesize these partial summaries into a single coherent summary"). Sequential only in v1; no parallel map calls.

### Decision 3: Hybrid retrieval as default for Document Q&A

1. Vector search over chunk embeddings (semantic).
2. FTS5 lexical search over chunk text (exact names, symbols, acronyms, dates).
3. Merge with weighted scoring (configurable weights; see §7).
4. Deduplicate by document and heading; apply per-document chunk cap.
5. Clamp `final_top_k` dynamically to fit context budget.
6. Pack into model context.

### Decision 4: SQLite + files for storage

- **SQLite** - structured app data: extracted text, chunks, messages, retrieval metadata, embeddings.
- **TOML** - human-editable config and prompt mode presets.
- **Plain files** - GGUF models, logs.
- **XDG-compliant paths** via the `directories` crate.

### Decision 5: Isolate `llama-cpp-2` behind `InferenceBackend`

`llama-cpp-2` follows llama.cpp closely and is not a stable idiomatic Rust API. All `llama-cpp-2` types must stay inside `llama-backend`. No leakage into `inference`, `core`, or `app-tui`.

---

## 4. Module Responsibilities and Core Traits

### `core`

Zero I/O. Zero DB. No external deps beyond `serde` and `uuid`. All other crates depend on `core`.

**Domain IDs:** `WorkspaceId`, `DocumentId`, `ConversationId`, `MessageId`, `ModelId`, `ModeId`, `ChunkId`.

**Domain types shared across crates:**

```rust
// Document parsing output — produced by doc-ingest, consumed by retrieval
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
    max_tokens: usize,
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

### `app` (binary - coordinator)

Owns references to all service crates. Starts the persistent inference worker thread and all background task workers. Holds `Sender<WorkerCommand>` for the inference worker. Receives `Command`s from `app-tui` and dispatches to the appropriate service. Forwards `AppEvent`s back to `app-tui`. The only crate that depends on everything.

### `app-tui`

Terminal setup/teardown, input handling, main event loop, ratatui rendering, mapping input to `Command`s. Manages transient UI state (focus, cursor, scroll, modal stack, prompt editor buffer). Depends only on `core` - no direct access to storage, inference, retrieval, or any service crate.

### `storage`

Opens and migrates the SQLite DB. Stores and retrieves workspaces, documents, document texts, chunks, conversations, messages, model registry, embedding metadata. Loads and saves TOML config. Resolves XDG directories. All writes that touch multiple tables use explicit transactions. Pure persistence - no business or retrieval logic.

### `doc-ingest`

Imports pasted text and `.txt`/`.md` files (later `.pdf`, `.epub`). Normalizes encodings via `encoding_rs`. Parses Markdown structure via `pulldown-cmark`. Produces `ParsedDocument` (defined in `core`). Does not chunk or write to the DB - hands `ParsedDocument` to the coordinator.

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
```

### `embeddings`

Pure computation crate. Loads an embedding model via `fastembed` (MiniLM/BGE-small class, ONNX via `ort`). Embeds arbitrary text; returns `Vec<f32>`. No knowledge of chunks, documents, or the database. Optionally supports reranking. Manages embedding model metadata (name, dimension, provider, version, path).

### `inference`

Backend-neutral API. Prompt assembly. Context budget calculation (see §6). Stream/cancel protocol. `StopReason` and `GenerationSummary` definitions. Does not own `llama-cpp-2` types.

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
    EndOfSequence,   // model produced EOS token naturally
    MaxTokens,       // hit max_tokens limit from ModeDefinition
    Cancelled,       // user-initiated cancellation
    Error(String),   // backend error mid-stream
}

struct GenerationSummary {
    model_id: ModelId,
    generated_tokens: usize,
    context_tokens_used: usize,
    tokens_per_second: f32,
    stop_reason: StopReason,
    duration_ms: u64,
}
```

### `llama-backend`

Owned by the persistent inference worker thread (see §6). Owns all `llama-cpp-2` types. Translates `ModelSpec` and `RuntimeModelConfig` into llama.cpp params. Applies context length, threads, batch size, mmap/mlock, and sampling parameters. Streams tokens via channel. Exposes tokens/sec and context usage. Supports cancellation. Handles model drop/reload on switch.

### `model-manager`

Scans model directories. Maintains local model registry in `storage`. Tracks and executes downloads. Validates files (hash check). Stores and loads per-model TOML settings. Provides model browser data to coordinator.

Scanned directories:
```
$XDG_DATA_HOME/LoreLM/models/
$HOME/.models/
custom paths from config.toml
```

---

## 5. Data Storage Schema

### XDG Directory Layout

```
$XDG_CONFIG_HOME/LoreLM/
  config.toml
  modes/
    document_qa.toml · summarizer.toml · editor_rewriter.toml · brainstorming.toml
  model-settings/<model-id>.toml

$XDG_DATA_HOME/LoreLM/
  app.db
  models/<repo-or-model-name>/model.gguf · metadata.toml
  document-snapshots/<document-id>.txt

$XDG_CACHE_HOME/LoreLM/downloads/
  embedding-models/ · temp/

$XDG_STATE_HOME/LoreLM/app.log
```

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

-- Non-content FTS5 table; stores its own copy of indexed text.
-- chunk_id is stored explicitly for join-back to chunks table.
-- No content= or content_rowid= to avoid TEXT PK / integer rowid mismatch.
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

-- INVARIANT: chunk_embeddings and chunk_vec_map must be written in a single
-- SQLite transaction. Partial failure must roll back both. Every chunk_vec_map
-- row must have a corresponding chunk_embeddings row.
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

**Re-import behavior:** When a file is re-imported, compare `content_hash`. If unchanged: no-op, inform user. If changed: delete the existing document row - `ON DELETE CASCADE` propagates to `document_texts`, `chunks`, `chunk_fts` (manual delete by `chunk_id`), `chunk_embeddings`, `chunk_vec_map`, `message_sources` - then re-import fresh. `chunk_fts` entries must be deleted manually before the chunk row is dropped since it is a virtual table without a foreign key.

---

## 6. Inference Pipeline

### Generation Flow

```
User submits prompt
  → create user message (status = 'pending') in DB
  → GenerationState transitions: Idle → Preparing
  → if mode RetrievalPolicy != None:
      GenerationState → Retrieving
      retrieve chunks via retrieval crate
  → GenerationState → Generating
  → build prompt: system + mode instructions + retrieved context
                  + recent history (excludes status = 'cancelled') + user prompt
  → send WorkerCommand::Generate to inference worker thread
  → stream TokenDelta events to TUI via AppEvent
  → on GenerationFinished: persist assistant message + source references
                           update user message status to 'complete'
  → GenerationState → Idle
```

### Context Budget Algorithm

> **IN-DEVELOPMENT** - this algorithm is directional. Implementation details may be refined.

```
fn compute_budget(
    context_size: usize,
    max_tokens: usize,          // from ModeDefinition.generation.max_tokens
    system_prompt: &str,
    history: &[Message],        // pre-filtered: excludes status = 'cancelled'
    retrieved_chunks: &[Chunk], // pre-clamped: final_top_k cap applied at retrieval time
    user_prompt: &str,
) -> Result<PackedContext, BudgetError> {

    let available = context_size - max_tokens;
    let system_tokens = estimate(system_prompt);
    let user_tokens = estimate(user_prompt);

    // Hard requirements — error if system + user prompt alone exceed budget
    if system_tokens + user_tokens > available {
        return Err(BudgetError::PromptTooLarge);
    }

    let remaining = available - system_tokens - user_tokens;

    // Fill retrieved chunks up to remaining budget
    let (chunks_used, chunk_tokens) = pack_chunks(retrieved_chunks, remaining);

    // Fill history with whatever remains, dropping oldest messages first
    let history_budget = remaining - chunk_tokens;
    let history_used = pack_history_newest_first(history, history_budget);

    Ok(PackedContext { system_prompt, user_prompt, chunks_used, history_used })
}
```

`final_top_k` is dynamically clamped before `retrieved_chunks` is passed to `compute_budget`: after model load, `ContextBudget` computes the available token budget and caps `final_top_k` to however many chunks fit. TOML values are maximums, not absolutes.

### Persistent Inference Worker Thread

The inference worker is a **persistent dedicated OS thread** (not tokio; not spawned per-generation) that owns the `llama-backend` instance for its entire lifetime. It loops on a `Receiver<WorkerCommand>` and processes commands sequentially, preventing concurrent inference.

The `app` coordinator holds `Sender<WorkerCommand>` and is the sole sender.

```rust
enum WorkerCommand {
    LoadModel(ModelSpec, RuntimeModelConfig),
    Generate(GenerateRequest, TokenSink, CancellationToken),
    UnloadModel,
    Shutdown,
}
```

Thread lifecycle: spawned once at app startup. `Shutdown` command triggers graceful exit. Model load/unload are explicit commands, not tied to generation lifetime.

```rust
enum GenerationEvent {
    Started { model_id: ModelId },
    TokenDelta { text: String },
    Stats { tokens_per_second: f32, generated_tokens: usize, context_used: usize },
    Finished { stop_reason: StopReason },
    Cancelled,
    Failed { error: String },
}
```

### Cancellation Policy

On user cancel (`Ctrl+C`):
1. `GenerationState` transitions `Generating → Cancelling`.
2. `CancellationToken` is signalled; inference worker stops generation.
3. On `GenerationCancelled` event received: both the pending user message and any partial assistant message are marked `status = 'cancelled'` in the `messages` table (soft delete).
4. `GenerationState` transitions `Cancelling → Idle`.

Cancelled messages are excluded from model context (history packing skips `status = 'cancelled'` rows) and hidden from the visible conversation view by default. The DB record is preserved.

### Model Switching

1. Cancel active generation (if any).
2. Send `WorkerCommand::UnloadModel` to inference worker.
3. Flush transient prompt/output UI state.
4. `ResourcePlanner::plan(new_model_spec, available_ram)` → new `RuntimeModelConfig`.
5. Send `WorkerCommand::LoadModel(spec, config)` to inference worker.
6. Keep workspace and conversation history intact.
7. Rebuild future prompts using new model's chat template and context size.

---

## 7. RAG and Retrieval Design

### Retrieval Flow

```
User query → normalize
  ├→ embed query via embeddings crate → vector search (top vector_top_k)
  └→ FTS5 query → lexical search (top fts_top_k)
       → merge with weighted scoring
       → deduplicate / apply max_chunks_per_document cap
       → clamp to dynamic final_top_k budget
       → pack into context via ContextBudget
       → build prompt → send to inference worker
```

### Hybrid Retrieval Config

```rust
struct HybridRetrievalConfig {
    vector_top_k: usize,            // default: 24
    fts_top_k: usize,               // default: 24
    final_top_k: usize,             // default: 8 (dynamic cap applied by ContextBudget)
    max_chunks_per_document: usize, // default: 3
    vector_weight: f32,             // default: 0.7
    fts_weight: f32,                // default: 0.3
    diversity_bonus: f32,           // default: 0.05 per unique document beyond first
    heading_match_bonus: f32,       // default: 0.1 if query terms appear in heading_path
}
```

Merge scoring:
```
score = vector_weight * normalized_vector_score
      + fts_weight * normalized_fts_score
      + diversity_bonus * (unique_document_rank - 1)
      + heading_match_bonus * heading_match(query, chunk.heading_path)
```

`max_chunks_per_document = 3` prevents a single large document from dominating retrieval. With `final_top_k = 8`, at least 3 distinct documents must contribute (assuming 3+ are indexed).

### Vector Index Trait

Wraps `sqlite-vec` (pre-v1 API) behind a stable interface:

```rust
trait VectorIndex {
    async fn upsert_chunk_embedding(&self, chunk_id: ChunkId, embedding: &[f32]) -> Result<()>;
    async fn search(&self, query: &[f32], limit: usize) -> Result<Vec<VectorHit>>;
}
```

---

## 8. State Management and Command/Event Pattern

### AppState and Sub-State Structs

All sub-state structs are defined in `core` alongside `AppState`.

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
    messages: Vec<Message>,           // loaded for display; excludes status = 'cancelled'
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
```

### GenerationState

```rust
enum GenerationState {
    Idle,
    Preparing,        // ContextBudget computation, prompt assembly
    Retrieving,       // RAG query in flight (retrieval modes only)
    Generating,       // tokens streaming from inference worker
    Cancelling,       // cancel signalled; awaiting GenerationCancelled event
    Failed(String),   // error message for display in status bar / modal
}
```

**Valid transitions:**
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

**Per-state TUI behavior:**
- `Idle`: prompt editor active; `Ctrl+C` inactive
- `Preparing` / `Retrieving`: spinner in status bar; prompt editor locked
- `Generating`: token stream visible; `Ctrl+C` active; status bar shows tokens/sec
- `Cancelling`: `Ctrl+C` inactive; spinner shows "Cancelling…"
- `Failed`: error displayed; prompt editor re-enabled for retry

### Command/Event Flow

```
Input event → Command (from app-tui)
  ├→ immediate AppState update (in app-tui)
  └→ dispatched to service crate by coordinator (app)
       → background work
       → AppEvent sent to app-tui
       → reducer updates AppState
```

Background workers communicate exclusively via `AppEvent`. They never mutate `AppState` directly. Workers: model downloads, document ingestion, chunking/indexing, embeddings, inference, exports.

---

## 9. Hardware-Specific Architecture Constraints

Target hardware: Intel Core Ultra 7 155H (16 cores, 22 threads), no discrete GPU, ~12 GB RAM available, Fedora. v1 is CPU-only - no Intel iGPU/NPU acceleration via `llama-cpp-2`.

### ResourcePlanner

```rust
struct ResourcePlannerInput {
    available_ram_bytes: u64,
    logical_threads: usize,
    model_size_bytes: u64,
    requested_context: Option<usize>,
}

struct RuntimeModelConfig {
    context_size: usize,
    threads: usize,
    batch_size: usize,
    ubatch_size: usize,
    use_mmap: bool,
    use_mlock: bool,
}
```

**Policy:** detect available RAM → estimate model footprint (weights + KV cache at requested context + output buffers + OS/app/SQLite/embedding model overhead) → reduce context size if necessary before refusing to load → warn user if result is `likely_swap`.

**Lifecycle - two trigger points:**

1. **App startup:** `ResourcePlanner::estimate_available_ram()` → stored in `AppState.available_ram_bytes`. Used to annotate model browser entries: `fits | tight | likely_swap`.
2. **Model load time:** `ResourcePlanner::plan(model_spec, available_ram)` → produces `RuntimeModelConfig` → coordinator sends `WorkerCommand::LoadModel(spec, config)`. User warning fires here if annotation is `likely_swap`.

**Default `RuntimeModelConfig`:**
```toml
context_size = 8192
threads = 8
batch_size = 512
ubatch_size = 128
use_mmap = true      # avoid eager anonymous copy of model weights
use_mlock = false    # don't pin RAM and starve the system
```

**Memory concurrency rule:**
```toml
[indexing]
pause_during_generation = true
max_parallel_embedding_batches = 1
```

---

## 10. Known Risks and Mitigations

| Risk | Mitigation |
|------|-----------|
| `llama-cpp-2` API instability | Hide behind `InferenceBackend`; pin crate versions; integration tests on the wrapper only |
| RAG quality disappoints | Hybrid retrieval with configurable weights; show retrieved sources in UI; add retrieval-debug view early; tune chunk sizes iteratively |
| Embedding model competes with LLM for RAM | Use small models (MiniLM/BGE-small); pause indexing during generation; unload embedder after bulk indexing |
| Model download scope creep | local scan → curated catalog → live search → gated model handling; each is a separate phase |
| PDF structure preservation is hard | Treat PDF as secondary; start page-level text only; heading heuristics later; `page_number` column on chunks from day one |
| Streaming UI state bugs | Keep output in transient buffer; render from immutable `AppState` snapshots; `GenerationState` enum enforces explicit state machine (see §8) |
| Context/retrieval budget mismatch | Default `context_size = 8192`; `ContextBudget` dynamically clamps `final_top_k` at runtime; TOML values are maximums not absolutes |
| Over-abstraction before first answer | Define traits; build vertical slice first (target: end of Phase 1); refactor after |
