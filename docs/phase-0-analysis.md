# LoreLM Phase 0 Analysis

## 1. Current Project Status

The current codebase implements the Phase 0 skeleton described in
`docs/implementation-plan.md` and marked complete in `docs/progress-tracker.md`.
The implemented vertical slice is intentionally small:

- Open a terminal UI.
- Resolve XDG-compliant application paths.
- Create or open the SQLite database.
- Create a default `config.toml` when one does not exist.
- Bootstrap a default workspace and conversation.
- Accept typed prompts in the TUI.
- Persist a user message and a stub assistant response.
- Reload non-cancelled message history after restart.

The next planned phase is Phase 1, "Minimal Local Inference". None of the real
inference, model loading, streaming generation, document import, retrieval,
embedding, model scanning, or download behavior is implemented yet. Those crates
exist with placeholder typed errors so the workspace shape and dependency
boundaries are present before feature work begins.

The checked-in workspace is clean. The current baseline validates with:

- `cargo check --workspace`
- `cargo test --workspace`

The test suite currently contains one behavior test in `storage`, covering
bootstrap, message persistence, and history reload.

## 2. Workspace Layout And Dependencies

The root `Cargo.toml` defines a Cargo workspace with resolver `2`, shared package
metadata, and shared dependency versions. It includes these workspace members:

- `app`
- `crates/app-tui`
- `crates/core`
- `crates/doc-ingest`
- `crates/embeddings`
- `crates/inference`
- `crates/llama-backend`
- `crates/model-manager`
- `crates/retrieval`
- `crates/storage`

This matches the modular monolith layout in `docs/architecture.md`. The current
dependency shape also broadly matches the intended boundaries:

- `app` is the binary crate and depends on every internal crate so it can become
  the coordinator.
- `app-tui` depends on `lorelm-core`, `ratatui`, `crossterm`, `tokio`, and
  `anyhow`.
- `lorelm-core` depends only on `serde` and `uuid`, keeping it free of I/O and
  database concerns.
- `storage` depends on `lorelm-core`, `rusqlite`, `directories`, `time`, `toml`,
  `serde`, `thiserror`, and `uuid`.
- Future feature crates have only the minimal dependencies needed to establish
  their planned relationships.

The active Phase 0 implementation lives in `app`, `app-tui`, `lorelm-core`, and
`storage`. The other crates are intentionally skeletal.

## 3. Runtime Flow

### Startup

`app/src/main.rs` owns the startup sequence:

1. `storage::AppPaths::resolve()` resolves XDG-compatible directories.
2. `logging::init(&paths)` creates the state directory and configures tracing to
   write to `app.log`.
3. `Storage::open(&paths)` creates directories, opens the SQLite database,
   enables foreign keys, and applies the schema.
4. `storage.load_config()` reads `config.toml` or writes default configuration.
5. `storage.bootstrap()` ensures a default workspace and conversation exist.
6. `storage.load_app_state(...)` loads the workspace, conversation, messages,
   and initial render state.
7. `tokio::sync::mpsc` channels are created for commands and events.
8. A `Coordinator` is moved onto a dedicated OS thread.
9. `app_tui::run(...)` starts the terminal UI on the async main task.

The coordinator uses `blocking_recv` and `blocking_send` because it runs on a
standard thread while the TUI uses async channel sends.

### Prompt Submission

Prompt handling starts in `crates/app-tui/src/lib.rs`:

1. `run` polls terminal events every 50 ms.
2. `handle_key` appends printable characters to the prompt buffer.
3. `Alt+Enter` inserts a newline.
4. `Enter` trims the prompt and, if non-empty, sets generation state to
   `Preparing`, clears the prompt, and sends `Command::SubmitPrompt`.
5. `Coordinator::run` receives the command and calls `persist_stub_exchange`.
6. `persist_stub_exchange` asks storage for the next message sequence.
7. A user `Message` is inserted and emitted as `AppEvent::MessageAppended`.
8. A stub assistant `Message` with content `Stub response: {content}` is inserted
   and emitted the same way.
9. The TUI receives each event in `run`, applies it with `apply_event`, and
   renders the updated transcript.

This proves the command/event architecture without requiring an inference
backend.

### Cancellation And Shutdown

`Ctrl+C` sends `Command::CancelGeneration`. In Phase 0 the coordinator only emits
`AppEvent::GenerationCancelled`; no in-flight worker exists. The TUI responds by
setting generation state back to `Idle`.

`Ctrl+Q` sends `Command::Quit` and returns from the TUI loop. The coordinator
thread exits when it receives `Quit`. `TerminalSession::drop` restores raw mode,
leaves the alternate screen, and shows the cursor.

## 4. File, Function, And Type Analysis

### `Cargo.toml`

The root manifest establishes the workspace and centralizes dependency versions.
This is important because later feature crates will add heavier dependencies
such as llama.cpp bindings, embedding runtimes, HTTP clients, and file parsers.
Keeping versions in `[workspace.dependencies]` gives all crates one source of
truth.

### `app/Cargo.toml`

The binary crate is named `lorelm`. It currently depends on all internal crates,
even though Phase 0 directly uses only `app-tui`, `lorelm-core`, and `storage`.
This anticipates the coordinator role described in `docs/architecture.md`, where
the binary will eventually wire ingestion, retrieval, inference, model
management, and embeddings.

### `app/src/main.rs`

`main` is the application entrypoint and is annotated with `#[tokio::main]`.
Its main responsibility is orchestration, not business logic:

- Resolve paths.
- Initialize logging.
- Open storage.
- Load config.
- Bootstrap durable state.
- Load initial `AppState`.
- Create command and event channels.
- Start the coordinator thread.
- Run the TUI.
- Join the coordinator thread and log any coordinator error.

The loaded `AppConfig` is passed into `Storage::load_app_state`, but that method
currently ignores it. This is acceptable for Phase 0 because config precedence
and model/runtime behavior are later-phase work.

### `app/src/logging.rs`

`init` configures structured logging:

- Ensures the XDG state directory exists.
- Creates a non-rolling `app.log` file appender with
  `tracing_appender::rolling::never`.
- Wraps the appender in a non-blocking writer.
- Uses `RUST_LOG` if present, otherwise defaults to `lorelm=info`.
- Initializes `tracing_subscriber::fmt`.
- Returns a `WorkerGuard`, which `main` holds so buffered log writes are flushed.

This implements the Phase 0 structured logging requirement.

### `app/src/coordinator.rs`

`Coordinator` owns:

- `storage: Storage`
- `command_rx: mpsc::Receiver<Command>`
- `event_tx: mpsc::Sender<AppEvent>`

`Coordinator::new` stores those dependencies.

`Coordinator::run` is the command dispatch loop. It handles:

- `Command::SubmitPrompt` by persisting a stub exchange.
- `Command::Quit` by exiting the loop.
- `Command::CancelGeneration` by emitting `GenerationCancelled`.
- UI-only commands as no-ops for now.

`persist_stub_exchange` is the current coordinator behavior:

- Reads the next sequence number from storage.
- Constructs a complete user message.
- Inserts the user message.
- Emits `MessageAppended`.
- Constructs a complete assistant message with a stub response.
- Inserts the assistant message.
- Emits `MessageAppended`.

This function connects the TUI command path to durable message history. In Phase
1, this stub exchange will be replaced or expanded by a real inference worker
flow.

### `crates/app-tui/Cargo.toml`

The TUI crate depends only on `lorelm-core` among internal crates, matching the
architecture rule that the frontend should communicate through commands, events,
and state rather than calling storage or services directly.

### `crates/app-tui/src/lib.rs`

`run` owns the interactive terminal loop:

- Starts `TerminalSession`.
- Stores mutable `AppState`.
- Stores the in-memory prompt buffer.
- Drains pending app events with `try_recv`.
- Renders the full screen.
- Polls crossterm events.
- Delegates key input to `handle_key`.

`apply_event` is the reducer for coordinator and worker events. In Phase 0 it
actively handles:

- `MessageAppended`: append to transcript and set generation state to `Idle`.
- `GenerationCancelled`: set generation state to `Idle`.
- `Error`: set generation state to `Failed` and store the UI error.

The remaining event variants are placeholders for later phases.

`handle_key` maps keyboard input to local state updates or commands:

- `Ctrl+Q`: send `Quit` and exit the TUI loop.
- `Ctrl+C`: send `CancelGeneration`.
- `Alt+Enter`: insert a newline.
- `Enter`: submit a non-empty prompt to the active conversation.
- `Backspace`: remove the last character from the prompt.
- Plain or shifted character input: append to the prompt.
- `Tab`: cycle focus between sidebar, transcript, and prompt.
- `PageUp` and `PageDown`: adjust transcript scroll offset.

The scroll offset is stored but not yet applied during rendering.

`render` builds the Phase 0 screen:

- A one-line header with workspace, mode, and model.
- A left sidebar showing placeholder document and session sections.
- A chat transcript showing role labels and message content.
- A prompt box, labelled as locked when generation is not idle or failed.
- A status line showing generation state and quit key.

`TerminalSession` wraps terminal setup and cleanup:

- `start` enables raw mode, enters the alternate screen, creates the crossterm
  backend, and initializes the ratatui terminal.
- `draw` delegates to `Terminal::draw`.
- `Drop` restores the terminal state.

### `crates/core/Cargo.toml`

`lorelm-core` is intentionally minimal: it uses `serde` for serializable shared
types and `uuid` for stable IDs. It has no I/O, database, terminal, or async
dependencies.

### `crates/core/src/ids.rs`

`id_type!` generates UUID-backed newtypes for:

- `WorkspaceId`
- `DocumentId`
- `ConversationId`
- `MessageId`
- `ModelId`
- `ChunkId`

Each generated ID type supports:

- `new` for random UUID generation.
- `from_uuid` for wrapping an existing UUID.
- `as_uuid` for unwrapping.
- `Default`, implemented as a new random ID.
- `Display`, using the UUID string.
- `FromStr`, parsing from a UUID string.
- `Serialize` and `Deserialize`.

`ModeId` is a string-backed ID rather than UUID-backed because modes are named
configuration concepts such as `freeform` and `document_qa`. It supports:

- `ModeId::named`.
- `as_str`.
- `Display`.
- `Serialize` and `Deserialize`.

### `crates/core/src/lib.rs`

This file defines the shared application vocabulary.

`AppError` is user-facing error data. `AppError::new` wraps any displayable
message into a serializable error struct.

`Command` is the TUI-to-coordinator command enum:

- `SubmitPrompt`
- `CancelGeneration`
- `ScrollTranscript`
- `SetFocus`
- `OpenScreen`
- `DismissError`
- `Quit`

Only `SubmitPrompt`, `CancelGeneration`, and `Quit` currently have coordinator
behavior.

`AppEvent` is the coordinator/worker-to-TUI event enum:

- `Tick`
- `MessageAppended`
- `DocumentImportProgress`
- `ModelDownloadProgress`
- `ModelLoaded`
- `RetrievalFinished`
- `TokenDelta`
- `GenerationFinished`
- `GenerationCancelled`
- `Error`

Only message append, cancellation, and error events currently affect TUI state.
The other variants reserve the event vocabulary needed for future phases.

`AppState` is the complete render model:

- `workspace: WorkspaceState`
- `conversation: ConversationState`
- `documents: DocumentPanelState`
- `models: ModelPanelState`
- `generation: GenerationState`
- `active_mode: ModeDefinition`
- `ui: UiState`
- `available_ram_bytes: u64`

The TUI renders from this state and mutates transient fields in response to local
input and app events.

`WorkspaceState` stores the active workspace.

`ConversationState` stores the active conversation, visible non-cancelled
messages, and transcript scroll offset.

`DocumentPanelState` stores known documents and active import tasks. It is empty
in Phase 0.

`ModelPanelState` stores the active model, installed models, and download tasks.
It is empty in Phase 0.

`UiState` stores active screen, focused panel, modal stack, and last error.
`Default` starts on the chat screen with prompt focus and no modal/error.

`Screen`, `Panel`, and `Modal` define the current UI navigation vocabulary.

`GenerationState` defines the planned lifecycle:

- `Idle`
- `Preparing`
- `Retrieving`
- `Generating`
- `Cancelling`
- `Failed`

Only `Idle`, `Preparing`, `Failed`, and cancellation reset behavior are used in
Phase 0.

`Workspace`, `Conversation`, and `Message` are durable conversation domain
models. `Message::new` creates a new message with a generated ID and empty
timestamp; storage fills the timestamp on insert.

`MessageRole` maps to database strings through `as_str`:

- `system`
- `user`
- `assistant`

`MessageStatus` maps to database strings through `as_str`:

- `complete`
- `cancelled`
- `error`

`DocumentSummary`, `ImportTask`, `DownloadTask`, and `RetrievalResult` are
placeholder UI/data-transfer structs for upcoming phases.

`LocalModel` and `ModelProvider` provide minimal model metadata. The architecture
document describes a richer model struct for later phases.

`GenerationSummary` and `StopReason` reserve the shape of finished generation
metadata, but they are not persisted yet.

`ParsedDocument`, `DocumentSource`, `DocumentBlock`, `DocumentBlockKind`, and
`SourceSpan` define the intended document ingestion output shape. The
`doc-ingest` crate does not produce these yet.

`ModeDefinition` contains mode metadata, prompt text, retrieval policy,
generation defaults, and conversation context policy. `ModeDefinition::freeform`
returns the built-in Phase 0 freeform mode.

`RetrievalPolicy`, `GenerationConfig`, and `ContextPolicy` support the planned
mode system.

### `crates/storage/Cargo.toml`

The storage crate owns persistence and XDG path resolution. It depends on core
domain types and persistence-related libraries only.

### `crates/storage/src/paths.rs`

`AppPaths` stores four root directories:

- `config_dir`
- `data_dir`
- `cache_dir`
- `state_dir`

`AppPaths::resolve` uses `directories::BaseDirs` and appends `LoreLM` to the
appropriate XDG locations. If a state directory is unavailable, it falls back to
a `state` directory under the local data directory.

`AppPaths::from_roots` supports tests and embedded launches by accepting explicit
directory roots.

`ensure_all` creates all top-level directories.

Accessor methods expose the directory paths, and helper methods return:

- `config_file`: `<config_dir>/config.toml`
- `database_file`: `<data_dir>/app.db`

### `crates/storage/src/lib.rs`

`StorageError` is the typed storage error enum. It covers directory resolution,
I/O, SQLite, TOML parsing, timestamp formatting, and UUID parsing.

`AppConfig` is the global application config loaded from `config.toml`. Its
default includes:

- model search paths
- basic UI preferences
- global inference runtime defaults
- retrieval defaults
- indexing defaults
- chunking defaults

The config shape is broader than Phase 0 uses, but it mirrors the planned v1
configuration in the design and architecture docs.

`PathConfig`, `UiConfig`, `InferenceConfig`, `InferenceDefaults`,
`RetrievalConfig`, `IndexingConfig`, and `ChunkingConfig` are nested config
sections.

`Bootstrap` returns the active workspace and conversation IDs created or found
during startup.

`Storage` owns:

- `paths: AppPaths`
- `connection: rusqlite::Connection`

`Storage::open` creates directories, opens SQLite, enables foreign keys, applies
the schema, and returns a storage handle.

`load_config` writes a default TOML file if none exists, then returns the config.
If the file exists, it parses TOML into `AppConfig`.

`bootstrap` creates or reuses the first workspace and first conversation within
that workspace. The default workspace is named `default`, and the default
conversation is titled `main` with active mode `freeform`.

`load_app_state` loads the active workspace, active conversation, and
non-cancelled messages. It constructs empty document/model panel state, sets
generation to `Idle`, builds a hard-coded freeform mode, initializes default UI
state, and sets `available_ram_bytes` to `0`.

`next_message_sequence` returns one greater than the current maximum message
sequence for a conversation, defaulting to `0`.

`insert_message` writes a message row. If the message timestamp is empty, it
generates an RFC3339 UTC timestamp. It also updates the parent conversation's
`updated_at` timestamp.

`migrate` executes the static `SCHEMA` SQL.

`load_workspace`, `load_conversation`, and `load_messages` are private read
helpers that convert SQLite rows into core domain structs.

`parse_id`, `parse_role`, and `parse_status` are row conversion helpers.

`now` formats the current UTC timestamp as RFC3339.

`SCHEMA` creates the Phase 0 database shape for all planned domains:

- workspaces
- documents
- document_texts
- chunks
- `chunk_fts`
- embedding_models
- chunk_embeddings
- chunk_vec_map
- conversations
- messages
- message_sources
- models
- model_downloads

As noted in `progress-tracker.md`, the FTS5 table is created in Phase 0, while
the actual `sqlite-vec` virtual table is deferred until Phase 4.

The storage unit test `bootstrap_persists_stub_messages_and_reloads_history`
creates isolated temporary roots, opens storage, loads config, bootstraps state,
inserts a user message, reopens storage, reloads app state, and verifies that the
message survived.

### `crates/doc-ingest`

`doc-ingest` is a Phase 0 stub. It currently defines only `DocIngestError` with
a `NotImplemented` variant. Its future role is to parse pasted text, `.txt`,
Markdown, and later PDF/EPUB sources into `ParsedDocument`.

### `crates/embeddings`

`embeddings` is a Phase 0 stub. It defines `EmbeddingError::NotImplemented`.
Its future role is pure compute: text in, embedding vector out, with no database
knowledge.

### `crates/inference`

`inference` is a Phase 0 stub. It defines `InferenceError::NotImplemented`.
Phase 1 will add the backend-neutral `InferenceBackend` trait, request/response
types, worker commands, generation events, and context budgeting types.

### `crates/llama-backend`

`llama-backend` is a Phase 0 stub. It defines
`LlamaBackendError::NotImplemented`. Later it will contain all `llama-cpp-2`
types and implement the backend-neutral inference interface.

### `crates/model-manager`

`model-manager` is a Phase 0 stub. It defines
`ModelManagerError::NotImplemented`. Later it will own model scanning, resource
planning, metadata, catalog loading, and download management.

### `crates/retrieval`

`retrieval` is a Phase 0 stub. It defines `RetrievalError::NotImplemented`.
Later it will own chunking, FTS indexing/querying, vector indexing, hybrid
retrieval, scoring, and context packing.

## 5. Architecture Alignment

The code aligns well with the Phase 0 deliverable:

- The workspace has all planned crates.
- Core IDs and placeholder domain types exist.
- `Command`, `AppEvent`, `AppState`, and sub-state structs exist.
- The ratatui/crossterm main loop exists.
- The coordinator skeleton and command/event channel wiring exist.
- SQLite schema creation and XDG path resolution exist.
- Config loading and default config creation exist.
- Structured logging is wired to the XDG state path.
- Stub message persistence and history reload exist.

The implementation also keeps the intended crate boundaries:

- `core` has no I/O dependencies.
- `app-tui` depends only on `core` among internal crates.
- `storage` owns persistence and config loading.
- `app` wires the pieces together.

Important current limitations are expected for Phase 0:

- There is no real inference worker.
- `TokenDelta` and `GenerationFinished` events are unused.
- Cancellation has no persisted soft-delete behavior yet.
- Document import, retrieval, embeddings, model scanning, and downloads are not
  implemented.
- The TUI displays placeholder document/session/model information.
- Config is loaded but not meaningfully applied to runtime state.

Small implementation observations:

- `Storage::load_app_state` duplicates the `ModeDefinition::freeform` literal
  instead of calling the existing constructor.
- `load_app_state` accepts `AppConfig` but ignores it.
- `load_messages` reads `model_id` from SQL but currently sets
  `Message.model_id` to `None`.
- Transcript scroll offset is mutated by keybindings but not applied in
  rendering.
- `app` depends on future service crates before it uses them.

These are not blockers for Phase 0, but they are useful cleanup candidates before
or during Phase 1.

## 6. YAGNI Refactor Suggestions

These suggestions apply the YAGNI principle: keep the implementation only as
large as the next milestone needs.

1. Use `ModeDefinition::freeform()` in `Storage::load_app_state`.

   The freeform mode is currently defined twice: once in `core` and once inline
   in `storage`. Calling the constructor removes duplication without adding a new
   abstraction.

2. Defer broader config plumbing until it affects behavior.

   `AppConfig` is already loaded, but most fields are for later phases. Avoid
   building a full config precedence or runtime settings system until Phase 1 or
   Phase 6 needs it. For now, either keep the unused parameter explicitly named
   `_config` or only thread through the specific Phase 1 model path when that
   work starts.

3. Avoid growing placeholder crates before their phase begins.

   The stub crates are enough to prove workspace structure. Do not add traits,
   DTOs, or module trees to `doc-ingest`, `retrieval`, `embeddings`,
   `model-manager`, or `llama-backend` until the implementation plan reaches the
   phase that needs them. The exception is `inference`, which is first in Phase 1
   and should get the minimal backend-neutral API required by the vertical slice.

4. Trim or tolerate unused binary dependencies intentionally.

   The `app` crate currently depends on all internal crates to match the planned
   coordinator role. If compile time or warnings become a problem, remove unused
   internal dependencies and add them back when wiring begins. If not, leaving
   them is acceptable because it documents intended ownership.

5. Keep TUI placeholders passive until real workflows exist.

   The sidebar, document panel state, model panel state, modal stack, and screen
   enum are useful scaffolding. Avoid implementing navigation and state mutation
   for file import, model library, settings, or mode editor screens until the
   relevant phase adds actual data and behavior.

6. Implement only the next slice of inference, not the full v1 abstraction.

   Phase 1 needs one local GGUF model, a persistent worker thread, streaming
   token events, cancellation, and assistant persistence. Avoid adding remote
   backend shape, catalog metadata, prompt editor behavior, or hybrid retrieval
   policy logic during that work.

7. Add tests around behavior, not placeholders.

   The existing storage test is useful because it exercises real Phase 0
   behavior. Continue that pattern: add tests for sequence numbering,
   cancellation persistence, worker event transitions, config parsing, or
   message reload when those behaviors exist. Avoid tests that only assert
   placeholder `NotImplemented` errors.

8. Fix small duplication before it spreads.

   The best low-cost cleanup is consolidating freeform mode construction and
   preserving `model_id` in `load_messages` once generation starts assigning it.
   Both changes keep the code simpler without committing to later-phase designs.
