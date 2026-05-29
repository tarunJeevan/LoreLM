
# LoreLM — Design Document

## 1. Project Overview

LoreLM is a local-first, workspace-centric TUI application for document Q&A, summarization, rewriting, brainstorming, and freeform text generation. Everything runs locally - models, documents, embeddings, and conversation history.

The central unit is a **workspace/project**. LoreLM is closer to a local document workspace than a chatbot.

### Primary Goals

1. Manage local GGUF models.
2. Ingest plain text and Markdown documents.
3. Ask questions over workspace documents with hybrid RAG.
4. Stream local model output into a `ratatui` TUI.
5. Preserve workspace and session history across restarts.
6. Export full transcripts as Markdown or plain text.
7. Architecture clean enough to add PDF, EPUB, remote backends, and richer RAG later.

### v1 Non-Goals

Do not build these first (leave architectural room, but do not block the prototype):

- Agentic workflows
- Plugin system
- Git-aware codebase Q&A
- Full PDF layout reconstruction
- Full Hugging Face model search
- Remote backend support
- OpenAI-compatible server mode
- Prompt templating system
- Advanced privacy/security controls beyond "everything is local"

---

## 2. Mode System

Modes are data-driven. Each mode defines its retrieval policy, system prompt, and generation defaults. Runtime settings (context_size, threads, etc.) are forbidden in mode TOML - those belong in global config or per-model TOML only.

### Built-in Modes

1. `document_qa` - answer questions using workspace documents
2. `summarizer` - compress documents faithfully; map-reduce for large docs
3. `editor_rewriter` - transform selected/pasted text
4. `brainstorming` - expansive generation, optional context attachment
5. `freeform` - no retrieval, no constraints

### Mode Definition

Canonical typed structs (defined in `core`):

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
    max_tokens: usize,  // caps generation length AND is the value reserved by ContextBudget
}

struct ContextPolicy {
    include_recent_messages: bool,
    max_recent_messages: usize,
}
```

### Example Mode TOML (`document_qa.toml`)

```toml
id = "document_qa"
name = "Document Q&A"
description = "Answer questions using documents in the current workspace."

retrieval_policy = "hybrid"
require_sources = true

[defaults]
temperature = 0.2
top_p = 0.9
repeat_penalty = 1.1
max_tokens = 768

[context]
include_recent_messages = true
max_recent_messages = 8

system_prompt = """
You answer questions using the provided document excerpts.
When possible, cite the source label, such as "From file notes/foo.md".
If the answer is not supported by the provided context, say so clearly.
"""
```

Mode presets live in TOML under `$XDG_CONFIG_HOME/LoreLM/modes/` - human-editable and versionable outside the database. All TOML fields map 1:1 to `ModeDefinition` fields. `max_tokens` serves as both the generation cap and the output reservation in `ContextBudget`; there is no separate `reserve_output_tokens` field.

---

## 3. Document Ingestion Design

### v1 Supported Formats

- Pasted text
- `.txt`
- `.md` / `.markdown`

### Secondary (Phase 7)

- `.pdf`
- `.epub`

### Tertiary (future)

- `.docx`, `.odt`, `.rtf`

### Import Pipeline

Each stage corresponds to a `documents.status` value. On failure at any stage, `status` is set to `failed` and `error_message` is populated.

```
User selects file or pastes text
  → check content_hash: if file already imported and hash unchanged → no-op, inform user
  → if hash differs: cascade-delete existing document and all dependent rows, re-import
  → detect source type                              [status: pending]
  → read bytes, decode (encoding_rs for non-UTF-8)
  → parse structure (pulldown-cmark for Markdown)  [status: parsed]
  → produce ParsedDocument (defined in core)
  → persist document + extracted text to storage
  → chunk document via retrieval crate             [status: chunked]
  → index chunks in FTS5 (chunk_fts table)         [status: indexed]
  → embed chunks via embeddings crate              [status: embedded]
  → insert vectors into chunk_vec / chunk_vec_map  [status: ready]
```

### Re-import Behavior

When a file is re-imported, `content_hash` is compared. If unchanged: no-op. If changed: the existing document row is deleted - `ON DELETE CASCADE` propagates to `document_texts`, `chunks`, `chunk_embeddings`, `chunk_vec_map`, and `message_sources`. `chunk_fts` entries must be deleted manually by `chunk_id` before the chunk row is dropped (virtual table, no FK). Then re-import proceeds from scratch.

### Markdown Parsing

Use `pulldown-cmark`. Preserve heading hierarchy as `heading_path`. Treat code blocks as atomic. Keep list items together. Keep tables together unless too large. Store both normalized plain text and structure metadata.

### Plain Text Parsing

Infer structure by: blank-line paragraph separation, heuristic headings (short lines, underlines, all-caps, accidental `#`). Store filename as source label.

### Chunking Strategy

Structure-aware:
1. Split document into sections by heading.
2. Split large sections by paragraph.
3. Split oversized paragraphs by sentence or token estimate.
4. Add overlap only between adjacent chunks in the same section.
5. Preserve chunk metadata: document ID, source label, heading path, byte range, page number (if known), token estimate.

```toml
[chunking]
target_tokens = 500
max_tokens = 800
overlap_tokens = 80
min_tokens = 80
```

### PDF Strategy (Phase 7, phased)

- **Phase A:** text by page, page-number source labels.
- **Phase B:** heading heuristics from font size/layout.
- **Phase C:** tables and references.

`page_number` column on chunks is present from day one, even before PDF support.

### EPUB Strategy (Phase 7)

EPUB is XHTML organized by spine and table of contents - maps naturally into document sections. Use a Rust EPUB crate to read metadata and spine content.

---

## 4. Model Management Design

### Local Model Registry

```rust
struct LocalModel {
    id: ModelId,
    display_name: String,
    provider: ModelProvider,
    local_path: Option<PathBuf>,       // None for undownloaded remote models
    repo_id: Option<String>,           // e.g. "Qwen/Qwen2.5-3B-Instruct-GGUF"
    filename: Option<String>,          // specific GGUF filename within repo
    size_bytes: Option<u64>,
    quantization: Option<String>,
    architecture: Option<String>,
    context_train: Option<usize>,
    chat_template: Option<String>,
    discovered_from: ModelDiscoverySource,
    installed_at: Option<DateTime>,
    last_used_at: Option<DateTime>,
}

enum ModelProvider {
    LocalGguf,
    HfGguf,
    RemoteOpenAiCompatible,
}
```

`ModelDiscoverySource` captures how the model was found: `directory_scan | curated_catalog | manual_path`.

### ResourcePlanner and Model Fit Annotation

At app startup, `ResourcePlanner::estimate_available_ram()` populates `AppState.available_ram_bytes`. Each model in the browser is annotated:

| Annotation | Meaning |
|---|---|
| `fits` | Model + KV cache at default context comfortably within available RAM |
| `tight` | Fits but leaves little headroom; consider reducing context size |
| `likely_swap` | Estimated footprint exceeds available RAM; user warned before load |

At model load time, `ResourcePlanner::plan(model_spec, available_ram)` produces a `RuntimeModelConfig`. If the annotation is `likely_swap`, the user is warned and must confirm before loading proceeds.

### Per-Model Settings (TOML)

Stored at `$XDG_CONFIG_HOME/LoreLM/model-settings/<model-id>.toml`. Only runtime and per-mode generation overrides live here - no retrieval policy, no context window policy.

```toml
model_id = "qwen2.5-3b-instruct-q4"
display_name = "Qwen 2.5 3B Instruct Q4"

[inference]
context_size = 8192
threads = 8
batch_size = 512
ubatch_size = 128
use_mmap = true
use_mlock = false

[defaults.document_qa]
temperature = 0.2
top_p = 0.9
repeat_penalty = 1.1
max_tokens = 768

[defaults.brainstorming]
temperature = 0.8
top_p = 0.95
repeat_penalty = 1.05
max_tokens = 1024
```

### Model Browser/Library UI

Tabs: `Installed | Downloads | Remote Catalog | Settings`

Model entries in the browser display the `fits | tight | likely_swap` annotation from `ResourcePlanner`.

Start with a curated catalog TOML:

```toml
[[models]]
name = "Example Small Instruct Model"
repo = "some/repo"
filename = "model.Q4_K_M.gguf"
size_hint = "2.5 GB"
recommended_ram_gb = 6
tags = ["small", "instruct", "qa"]
```

Live HF search via `hf-hub` is a later-phase addition.

### Download Manager

Background task managed by `model-manager`. State machine:

```
queued
  → downloading          (worker picks up task)
      → complete         (all bytes received, hash validated, moved to models dir)
      → failed           (network error, hash mismatch, disk full)
      → cancelled        (user cancelled mid-download)

failed    → [prompt user: retry or quit?]
    retry (confirmed)  → queued
    quit  (confirmed)  → terminal_failed     (no further transitions)

cancelled → [prompt user: restart or quit?]
    restart (confirmed) → queued
    quit    (confirmed) → terminal_cancelled  (no further transitions)

complete          → terminal
terminal_failed   → terminal
terminal_cancelled → terminal
```

**Rules:**
- Partial files live in `$XDG_CACHE_HOME/LoreLM/downloads/temp/` during download.
- On any exit from `downloading` other than `complete`: partial file deleted; `bytes_downloaded` reset to 0.
- On `complete`: file moved to `$XDG_DATA_HOME/LoreLM/models/`; `models` row inserted; `model_downloads` row updated.
- Retry and restart require explicit user confirmation - no automatic retry.
- `model_downloads` row retained in DB for all terminal states.
- Resume (partial download pickup) is out of scope for v1 - retry always restarts from zero.

### Recommended Models for Target Hardware

| Model size | Quantization | Fit expectation                          |
|------------|--------------|------------------------------------------|
| 1.5B–3B    | Q4/Q5/Q8     | Comfortable                              |
| 4B         | Q4/Q5        | Comfortable to good                      |
| 7B–8B      | Q4           | Usable; context size matters             |
| 7B–8B      | Q8           | Likely too memory-heavy at large context |
| 13B+       | Q4           | Not a v1 target on 12 GB available RAM  |

Start with a 3B–4B instruct model in Q4 or Q5. Test 7B/8B Q4 after the pipeline is proven.

---

## 5. TUI Layout and Navigation

### Main Layout

```
┌────────────────────────────────────────────────────────────────────┐
│ Workspace: research-notes        Mode: Document Q&A   Model: 3B Q4 │
├──────────────────┬─────────────────────────────────────────────────┤
│ Documents        │ Chat / Session                                  │
│                  │                                                 │
│ [x] notes/foo.md │ User: ...                                       │
│ [x] paper.md     │ Assistant: ...                                  │
│ [ ] draft.txt    │                                                 │
│                  │ Sources:                                        │
│ Sessions         │ - notes/foo.md                                  │
│ > main           │ - paper.md                                      │
│   branch-1       │                                                 │
├──────────────────┴─────────────────────────────────────────────────┤
│ Prompt editor                                                      │
│ > Ask a question about the selected documents...                   │
├────────────────────────────────────────────────────────────────────┤
│ tokens/s: 12.4 | ctx: 3120/8192 | model: qwen-3b-q4 | indexing: ok │
└────────────────────────────────────────────────────────────────────┘
```

### Core Screens

**1. Chat screen** (primary daily-use screen)
Panes: workspace/doc sidebar, main transcript, collapsible sources panel, prompt editor, status bar.

**2. File picker/import screen**
Browse filesystem, multi-select files, preview selected file, import as document, show import/indexing progress with per-stage status.

**3. Model library screen**
Installed models (with fit annotation), downloadable models, active downloads with state machine status, per-model settings, load/unload/switch.

**4. Mode/prompt editor**
Select mode, edit system prompt, restore default, save mode preset, preview final prompt structure.

**5. Settings screen**
General app settings, storage paths, model directories, inference defaults, retrieval/chunking defaults, keybindings.

### Keybindings

```
Ctrl+O      Open/import file
Ctrl+M      Model library
Ctrl+P      Mode/prompt editor
Ctrl+,      Settings
Ctrl+E      Export transcript
Ctrl+C      Cancel generation
Ctrl+N      New conversation
Ctrl+D      Clone current conversation
Tab         Cycle focus
Shift+Tab   Reverse focus
PgUp/PgDn   Scroll transcript
Enter       Submit prompt
Alt+Enter   Newline in prompt
```

---

## 6. Configuration Design

### Config Precedence Stack

Settings are resolved at generation time. Lower layers are overridden by higher layers:

| Priority | Source | Scope |
|---|---|---|
| 1 (lowest) | `global config.toml [inference.defaults]` | All models and modes |
| 2 | Mode TOML `[defaults]` | This mode, all models |
| 3 | Per-model TOML `[defaults.<mode_id>]` | This mode, this model |
| 4 (highest) | Workspace TOML *(v2, not implemented)* | This project only |

### Settings Ownership

Each setting belongs to exactly one config layer:

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

### Global Config (`config.toml`)

```toml
[paths]
model_dirs = ["~/.models"]

[ui]
theme = "default"
show_sources_panel = true

[inference.defaults]
context_size = 8192
threads = 8
batch_size = 512
ubatch_size = 128
use_mmap = true
use_mlock = false

[retrieval]
strategy = "hybrid"
vector_top_k = 24
fts_top_k = 24
final_top_k = 8
max_chunks_per_document = 3
vector_weight = 0.7
fts_weight = 0.3
diversity_bonus = 0.05
heading_match_bonus = 0.1

[indexing]
pause_during_generation = true
max_parallel_embedding_batches = 1

[chunking]
target_tokens = 500
max_tokens = 800
overlap_tokens = 80
min_tokens = 80
```

---

## 7. Document Q&A Prompt Shape

```
[system]
You are a local document Q&A assistant.
Use only the provided document excerpts when answering document-specific questions.
Cite source labels like "From file notes/foo.md" when relevant.

[retrieved context]
<context>
[source: notes/foo.md | heading: Installation]
...chunk text...
</source>

[source: research/bar.md | heading: Limitations]
...chunk text...
</source>
</context>

[recent conversation]
User: ...
Assistant: ...

[current user question]
...
```

---

## 8. Crate Dependency List

### TUI and Input

| Crate | Use |
|-------|-----|
| `ratatui` | Core immediate-mode terminal UI |
| `crossterm` | Terminal input/events/backend |
| `tui-textarea` or custom | Multiline prompt editing |
| `unicode-width` | Correct wide-char layout |

### Async, Events, Concurrency

| Crate | Use |
|-------|-----|
| `tokio` | Async runtime |
| `async-trait` | Async backend traits |
| `crossbeam-channel` or `tokio::sync::mpsc` | Streaming tokens and progress events |
| `parking_lot` | Lightweight locks for local state |

### Error Handling and Logging

| Crate | Use |
|-------|-----|
| `thiserror` | Typed errors for internal crates |
| `anyhow` | Ergonomic application-level errors |
| `tracing` | Structured async-safe logging |
| `tracing-subscriber` | Log configuration |
| `tracing-appender` | Rolling log files to XDG state path |

### Config and Serialization

| Crate | Use |
|-------|-----|
| `serde` | Domain/config serialization |
| `toml` | Human-editable config and presets |
| `serde_json` | Flexible metadata blobs in SQLite |
| `directories` | XDG config/data/cache paths |
| `uuid` | Stable IDs for workspaces, docs, etc. |
| `time` | Timestamps (prefer over chrono unless needed) |

### Storage and Retrieval

| Crate | Use |
|-------|-----|
| `rusqlite` (bundled) | SQLite access |
| `refinery` or `barrel` | DB migrations |
| `sqlite-vec` | Vector storage/search (pre-v1; wrap behind `VectorIndex` trait) |
| `zerocopy` | Efficient embedding byte passing for sqlite-vec |
| SQLite FTS5 | Lexical full-text search (built into SQLite) |

### Embeddings and RAG

| Crate | Use |
|-------|-----|
| `fastembed` | Local embeddings and reranking via ONNX |
| `tokenizers` | Optional tokenizer utilities |
| `text-splitter` or custom | Chunking (custom Markdown-aware splitter recommended) |

### Inference

| Crate | Use |
|-------|-----|
| `llama-cpp-2` | Local GGUF inference (wrapped behind `InferenceBackend`) |
| `hf-hub` | Hugging Face Hub async/sync client and downloads |
| `reqwest` | Manual HTTP fallback |
| `sha2` | Model download checksum validation |

### Document Ingestion

| Crate | Use |
|-------|-----|
| `pulldown-cmark` | Markdown pull parser (CommonMark + extensions) |
| `walkdir` | Directory traversal for folder import |
| `ignore` | `.gitignore`-aware traversal |
| `encoding_rs` | Non-UTF-8 text encoding handling |
| `mime_guess` or `infer` | File type detection for import routing |
| `pdf-extract` | PDF text extraction by page (Phase 7) |
| `lib-epub` or `epub` | EPUB metadata and spine content (Phase 7) |
| `zip` + `quick-xml` | DOCX/ODT extraction, future tertiary format |

### Future Remote/Server Features

| Crate | Use |
|-------|-----|
| `async-openai` or custom reqwest | OpenAI-compatible backend (Phase 8) |
| `axum` | Future local API server (Phase 8) |
| `tower` | Middleware for server mode (Phase 8) |

---

## 9. Phased Implementation Roadmap

### Phase 0 - Skeleton App
Cargo workspace with `app` binary crate and all internal crates stubbed. `ratatui` main loop, `AppState`, command/event system, basic chat screen, prompt editor, config path resolution, SQLite creation and migrations, logging. No llama.cpp. No real inference.

**Deliverable:** TUI opens, accepts typed prompts, stores stub messages in SQLite, reloads history after restart. *(Delivers steps 1 and 10 of the vertical slice only - full slice requires Phase 1.)*

### Phase 1 - Minimal Local Inference
`InferenceBackend` trait, `llama-backend`, persistent inference worker thread with `WorkerCommand` enum, `ResourcePlanner` (both lifecycle points), load a manually path-configured GGUF, non-streaming generation, then streaming, then soft-delete cancellation, persist completed responses.

**Deliverable:** User can chat with one local GGUF model from the TUI. Full vertical slice executable end-to-end.

### Phase 2 - Workspace + Document Import + Model Scanning
Workspaces, file picker, import `.txt` and `.md`, parse Markdown headings, store extracted text, document sidebar with per-stage import status, source labels. **Also:** model directory scanning, installed model list with fit annotation, per-model TOML settings, model switching without restart.

**Deliverable:** User can create a workspace, import txt/md files, see them in the sidebar, and switch between locally installed models without restarting.

### Phase 3 - Document Q&A (FTS only)
Chunker, SQLite FTS5 non-content index, basic lexical retrieval, `ContextBudget` with dynamic `final_top_k` clamping, Document Q&A mode, file-level source references.

**Deliverable:** User asks a question; model answers using FTS-retrieved chunks from imported files with source citations.

### Phase 4 - Embeddings + Hybrid Retrieval
`fastembed` integration, embedding model registry, chunk embedding worker, `sqlite-vec` vector table, `chunk_embeddings`/`chunk_vec_map` transactional writes, query embedding, hybrid retrieval with configurable merge weights, retrieval diagnostics panel.

**Deliverable:** Document Q&A uses both semantic and lexical search.

### Phase 5 - Model Catalog + Download Manager
Curated remote model catalog (TOML-based), download manager with full state machine, user-confirmed retry/restart, progress display, hash validation, file lifecycle management.

**Deliverable:** User can browse a curated model catalog, download models with progress tracking, and retry or cancel downloads with confirmation.

### Phase 6 - Modes, Prompts, Exports, Cloning
Mode editor UI, saved system prompt presets, Summarizer mode (with map-reduce for large docs), Editor/Rewriter and Brainstorming modes, export transcript to Markdown/plain text, clone conversation.

**Deliverable:** User can work across multiple task modes and export useful artifacts.

### Phase 7 - PDF and EPUB

> **Note:** Phases 6 and 7 are independent tracks with no dependency on each other. They can be reordered or worked in parallel without breaking upstream or downstream dependencies.

PDF text-by-page import (Phase A), page source references, EPUB spine/TOC import, structure metadata, import error reporting with `error_message` column.

**Deliverable:** User can import PDFs and EPUBs with text extraction and basic source references.

### Phase 8 - Future Backend Abstraction
OpenAI-compatible backend, llama.cpp server backend, optional `axum` app server, remote model config UI.

**Deliverable:** Same TUI can use local llama.cpp or a remote OpenAI-compatible endpoint.

---

## 10. First Vertical Slice — Target: End of Phase 1

```
1.  Open TUI
2.  Load one local GGUF model from config (path hardcoded or from config.toml)
3.  Create/open a workspace
4.  Import one Markdown file
5.  Chunk it
6.  Search chunks with FTS
7.  Ask a question
8.  Build context prompt via ContextBudget
9.  Stream answer from inference worker thread
10. Save transcript (soft-delete cancelled messages; persist completed exchange)
```

Phase 0 delivers steps 1 and 10 only (TUI shell + SQLite history). Steps 2–9 require Phase 1 to be complete. Second slice: add embeddings and hybrid retrieval (Phase 4). Third slice: add model catalog and download manager (Phase 5).

---

## 11. Recommended v1 Feature Set

- XDG-compliant config/data/cache/state directories
- Workspace-based sessions with persistent linear chat history
- Conversation cloning
- Text and Markdown import; pasted text import with re-import detection
- Model scanning from XDG model dir and `$HOME/.models` with fit annotation
- Per-model TOML settings; model switching without restart
- Curated model download browser with confirmed retry/cancel
- Document Q&A mode with hybrid retrieval (FTS + vector) and source citations
- Summarizer mode with map-reduce for large documents
- Editor/Rewriter and Brainstorming modes
- Streaming output with soft-delete cancellation
- Status bar: tokens/sec, context usage (`ctx: used/total`), current model, indexing status
- Full transcript export to Markdown/plain text
- Structured logging to XDG state path
- Modular `InferenceBackend` trait with only `llama-backend` implemented in v1
