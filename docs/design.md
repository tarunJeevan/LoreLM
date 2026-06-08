# LoreLM — Design

## 1. Project Overview

LoreLM is a local-first, workspace-centric TUI application for document Q&A, summarization, rewriting, brainstorming, and freeform text generation. Everything runs locally — models, documents, embeddings, and conversation history.

The central unit is a **workspace/project**, not a model or chat window. LoreLM is closer to a local document workspace than a chatbot.

### Primary Goals

1. Manage local GGUF models.
2. Ingest plain text and Markdown documents.
3. Ask questions over workspace documents with hybrid RAG.
4. Stream local model output into a `ratatui` TUI.
5. Preserve workspace and session history across restarts.
6. Export full transcripts as Markdown or plain text.
7. Architecture clean enough to add PDF, EPUB, remote backends, and richer RAG later.

### v1 Non-Goals

Leave architectural room for these, but do not build them in v1:

- Agentic workflows
- Plugin system
- Git-aware codebase Q&A
- Full PDF layout reconstruction
- Full Hugging Face model search
- Remote backend support
- OpenAI-compatible server mode
- Prompt templating system
- Advanced privacy/security controls beyond "everything is local"

### Recommended v1 Feature Set

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
- Status bar: tokens/sec, context usage, current model, indexing status
- Full transcript export to Markdown/plain text
- Structured logging to XDG state path
- Modular `InferenceBackend` trait with only `llama-backend` implemented

---

## 2. Core User Flow

```
1. User opens LoreLM
2. User creates or opens a workspace
3. User selects or downloads a local GGUF model
4. User imports documents (txt, md) into the workspace
5. Documents are chunked, indexed, and embedded in the background
6. User selects a mode (Document Q&A, Summarizer, etc.)
7. User types a prompt
8. App retrieves relevant chunks (if mode uses retrieval)
9. App builds prompt and streams response from the local model
10. Response and source citations are displayed; exchange is persisted
11. User can export the conversation or clone it for a new branch
```

---

## 3. Mode System

Modes are data-driven, not hard-coded control flows. Each mode defines its retrieval policy, system prompt, and generation defaults. Runtime settings (`context_size`, `threads`, etc.) are forbidden in mode TOML — those belong in global config or per-model TOML only.

### Built-in Modes

| Mode | Retrieval | Description |
|---|---|---|
| `document_qa` | Hybrid (vector + FTS) | Answer questions using workspace documents |
| `summarizer` | Direct full-context or map-reduce | Compress documents faithfully |
| `editor_rewriter` | None (direct text) | Transform selected/pasted text |
| `brainstorming` | None (optional attachment) | Expansive generation |
| `freeform` | None | No retrieval, no constraints |

**Summarizer map-reduce:** If `document_token_estimate > context_size - max_tokens - system_prompt_tokens`, split via the RAG chunker, summarize each chunk sequentially (map: "Summarize this excerpt faithfully and concisely"), then synthesize (reduce: "Synthesize these partial summaries into a single coherent summary"). Sequential only in v1.

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

Mode presets live in TOML under `$XDG_CONFIG_HOME/LoreLM/modes/` — human-editable and versionable. `max_tokens` serves as both the generation cap and the output reservation in `ContextBudget`; there is no separate `reserve_output_tokens` field.

---

## 4. Document Ingestion

### Supported Formats

| Phase | Formats |
|---|---|
| v1 | Pasted text, `.txt`, `.md` / `.markdown` |
| Phase 7 | `.pdf`, `.epub` |
| Future | `.docx`, `.odt`, `.rtf` |

### Import Pipeline

Each stage corresponds to a `documents.status` value. On failure at any stage, `status` is set to `failed` and `error_message` is populated.

```
User selects file or pastes text
  → check content_hash: if unchanged → no-op, inform user  [existing file]
  → if hash differs: cascade-delete existing document rows, re-import
  → detect source type                                      [status: pending]
  → read bytes, decode (encoding_rs for non-UTF-8)
  → parse structure (pulldown-cmark for Markdown)           [status: parsed]
  → produce ParsedDocument
  → persist document + extracted text to storage
  → chunk document via retrieval crate                      [status: chunked]
  → index chunks in FTS5                                    [status: indexed]
  → embed chunks via embeddings crate                       [status: embedded]
  → insert vectors into chunk_vec / chunk_vec_map           [status: ready]
```

### Chunking Strategy

Structure-aware, starting from headings and working down to paragraphs and sentences:

1. Split document into sections by heading.
2. Split large sections by paragraph.
3. Split oversized paragraphs by sentence or token estimate.
4. Add overlap only between adjacent chunks in the same section.
5. Preserve metadata: document ID, source label, heading path, byte range, page number (if known), token estimate.

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

`page_number` column on `chunks` is present from day one.

---

## 5. Model Management

### Local Model Registry

```rust
struct LocalModel {
    id: ModelId,
    display_name: String,
    provider: ModelProvider,           // LocalGguf | HfGguf | RemoteOpenAiCompatible
    local_path: Option<PathBuf>,
    repo_id: Option<String>,
    filename: Option<String>,
    size_bytes: Option<u64>,
    quantization: Option<String>,
    architecture: Option<String>,
    context_train: Option<usize>,
    chat_template: Option<String>,
    discovered_from: ModelDiscoverySource, // directory_scan | curated_catalog | manual_path
    installed_at: Option<DateTime>,
    last_used_at: Option<DateTime>,
}
```

Scanned directories: `$XDG_DATA_HOME/LoreLM/models/`, `$HOME/.models/`, and any custom paths from `config.toml`.

### ResourcePlanner and Fit Annotation

At app startup, available RAM is estimated and stored in `AppState`. Each model is annotated:

| Annotation | Meaning |
|---|---|
| `fits` | Comfortably within available RAM at default context |
| `tight` | Fits but leaves little headroom |
| `likely_swap` | Estimated to exceed available RAM; user warned before load |

At model load time, `ResourcePlanner::plan(model_spec, available_ram)` produces a `RuntimeModelConfig`. If `likely_swap`, the user must confirm before loading proceeds.

### Per-Model Settings (TOML)

Stored at `$XDG_CONFIG_HOME/LoreLM/model-settings/<model-id>.toml`:

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

### Download Manager State Machine

```
queued
  → downloading
      → complete         (bytes received, hash validated, moved to models dir)
      → failed           (network error, hash mismatch, disk full)
      → cancelled        (user cancelled)

failed    → [prompt: retry or quit?]
    retry (confirmed)   → queued
    quit  (confirmed)   → terminal_failed

cancelled → [prompt: restart or quit?]
    restart (confirmed) → queued
    quit    (confirmed) → terminal_cancelled

complete / terminal_failed / terminal_cancelled → (no further transitions)
```

Rules: partial files live in `$XDG_CACHE_HOME/LoreLM/downloads/temp/`; deleted on any non-`complete` exit from `downloading`. Retry always restarts from zero — no resume in v1. `model_downloads` row retained in DB for all terminal states.

### Recommended Models for Target Hardware

Target: Intel Core Ultra 7 155H, ~12 GB RAM available, no discrete GPU.

| Model size | Quantization | Fit expectation |
|---|---|---|
| 1.5B–3B | Q4/Q5/Q8 | Comfortable |
| 4B | Q4/Q5 | Comfortable to good |
| 7B–8B | Q4 | Usable; context size matters |
| 7B–8B | Q8 | Likely too heavy at large context |
| 13B+ | Q4 | Not a v1 target |

---

## 6. TUI Layout and Navigation

### Layout Pattern

A persistent three-zone layout:

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

- **Header bar:** active workspace, mode, and model.
- **Left pane:** document list with import status; session list.
- **Main pane:** conversation transcript with collapsible sources panel.
- **Prompt editor:** multiline input; locked during generation.
- **Status bar:** live generation stats, context usage, indexing status.

### Screens

**1. Chat** (primary)
Workspace/doc sidebar, transcript, collapsible sources panel, prompt editor, status bar.

**2. File picker / import**
Filesystem browser, multi-select, file preview, per-stage import status display.

**3. Model library**
Tabs: `Installed | Downloads | Remote Catalog | Settings`. Model entries show fit annotation. Active downloads show state machine status.

**4. Mode / prompt editor**
Select mode, edit system prompt, restore default, save preset, preview final prompt structure.

**5. Settings**
App settings, storage paths, model directories, inference defaults, retrieval/chunking defaults, keybindings.

### Keybindings

```
Ctrl+O        Open/import file
Ctrl+M        Model library
Ctrl+P        Mode/prompt editor
Ctrl+,        Settings          (Ctrl+S avoided: sends XOFF on Linux TTYs)
Ctrl+E        Export transcript
Ctrl+C        Cancel generation
Ctrl+N        New conversation
Ctrl+D        Clone current conversation
Tab           Cycle focus
Shift+Tab     Reverse focus
PgUp/PgDn     Scroll transcript
Enter         Submit prompt
Alt+Enter     Newline in prompt
```

### Generation State and TUI Behavior

| State | Prompt editor | Ctrl+C | Status bar |
|---|---|---|---|
| `Idle` | Active | Inactive | — |
| `Preparing` | Locked | Inactive | Spinner |
| `Retrieving` | Locked | Inactive | Spinner |
| `Generating` | Locked | Active | tokens/sec |
| `Cancelling` | Locked | Inactive | "Cancelling…" |
| `Failed` | Active (retry) | Inactive | Error message |

---

## 7. Configuration

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

### Document Q&A Prompt Shape

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
