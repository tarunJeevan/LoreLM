# LoreLM — Implementation Plan

## Overview

The plan is structured in nine phases (0–8). Each phase has a goal, a set of concrete implementation steps, and a deliverable that can be verified by running the app.

**Milestone summary:**

| Milestone | Target phase |
|---|---|
| First vertical slice (meaningful prototype) | End of Phase 1 |
| Full Document Q&A with hybrid retrieval | End of Phase 4 |
| Complete v1 feature set | End of Phase 6 |
| PDF/EPUB support | Phase 7 (independent) |
| Remote backend abstraction | Phase 8 |

> **Note:** Phases 6 and 7 are independent tracks. They can be reordered or worked in parallel without breaking upstream or downstream dependencies.

---

## Phase 0 — Skeleton App

**Goal:** Prove the TUI architecture and establish the full workspace structure before any real inference or storage logic is written.

**Steps:**

1. Initialize the Cargo workspace with all crates stubbed (`app`, `app-tui`, `core`, `storage`, `doc-ingest`, `retrieval`, `embeddings`, `inference`, `llama-backend`, `model-manager`).
2. Define all domain IDs and placeholder domain types in `core`.
3. Define `AppEvent`, `Command`, and `AppState` with sub-state structs in `core`.
4. Implement the `ratatui` main loop in `app-tui`: terminal setup/teardown, input handling, basic chat screen render, prompt editor.
5. Implement the `app` coordinator skeleton: channel wiring between `app-tui` and service stubs, event dispatch loop.
6. Implement `storage`: SQLite creation, all migrations, XDG path resolution, basic workspace and message persistence.
7. Implement config loading from `config.toml` via `storage`.
8. Wire up structured logging (`tracing` + `tracing-appender`) to the XDG state path.
9. Implement stub message persistence: on prompt submit, write a fake assistant response to the DB and reload on restart.

**Deliverable:** TUI opens, accepts typed prompts, stores stub messages in SQLite, reloads history after restart.

> Phase 0 delivers steps 1 and 10 of the first vertical slice only. Steps 2–9 require Phase 1.

---

## Phase 1 — Minimal Local Inference

**Goal:** Complete the first vertical slice — a working local document Q&A loop, end to end.

**Steps:**

1. Define `InferenceBackend` trait and all related types (`ModelSpec`, `RuntimeModelConfig`, `GenerateRequest`, `GenerationSummary`, `StopReason`, `WorkerCommand`, `GenerationEvent`) in `inference`.
2. Implement `ResourcePlanner` in `model-manager`:
   - Startup: `estimate_available_ram()` → store in `AppState.available_ram_bytes`.
   - Load time: `plan(model_spec, available_ram)` → `RuntimeModelConfig`.
3. Implement the persistent inference worker thread in `app`: spawned once at startup, loops on `Receiver<WorkerCommand>`.
4. Implement `llama-backend`: wrap `llama-cpp-2`, translate `ModelSpec` + `RuntimeModelConfig` into llama.cpp params, non-streaming generation first.
5. Wire `app` coordinator: on `Command::Generate`, send `WorkerCommand::Generate` to inference worker; forward `GenerationEvent`s as `AppEvent`s to `app-tui`.
6. Add streaming: emit `AppEvent::TokenDelta` on each token; render incrementally in `app-tui`.
7. Implement `GenerationState` transitions in `app-tui`: `Idle → Preparing → Generating → Idle`.
8. Implement soft-delete cancellation: `Ctrl+C` sends cancel; marks user and assistant messages `status = 'cancelled'`.
9. Implement message persistence: on `GenerationFinished`, persist assistant message with `GenerationSummary` fields.
10. Load model path from `config.toml` (hardcoded path for now; no model browser yet).

**Deliverable:** User can chat with one local GGUF model from the TUI with streaming output and cancellation. Full vertical slice executable end-to-end.

---

### First Vertical Slice — Target: End of Phase 1

```
1.  Open TUI
2.  Load one local GGUF model from config.toml
3.  Create/open a workspace
4.  Import one Markdown file
5.  Chunk it
6.  Search chunks with FTS
7.  Ask a question
8.  Build context prompt via ContextBudget
9.  Stream answer from inference worker thread
10. Save transcript (soft-delete cancelled; persist completed exchange)
```

---

## Phase 2 — Workspace + Document Import + Model Scanning

**Goal:** Make the app workspace-aware, able to ingest documents, and able to switch between locally installed models.

**Steps:**

1. Implement workspace creation, selection, and persistence in `storage` and the `app` coordinator.
2. Implement the file picker screen in `app-tui`: filesystem browse, multi-select, preview.
3. Implement `doc-ingest`: plain text and Markdown import, `encoding_rs` normalization, `pulldown-cmark` parsing, `ParsedDocument` output.
4. Implement the import pipeline coordinator in `app`: trigger on file selection, emit `DocumentImportProgress` events per stage, update `documents.status`.
5. Render the document sidebar in `app-tui` with per-stage import status and error messages.
6. Implement model directory scanning in `model-manager`: scan `$XDG_DATA_HOME/LoreLM/models/`, `$HOME/.models/`, and custom paths; populate the `models` table.
7. Implement per-model TOML settings: load/save at `$XDG_CONFIG_HOME/LoreLM/model-settings/<model-id>.toml`.
8. Implement model switching in `app` coordinator: cancel generation → `WorkerCommand::UnloadModel` → `ResourcePlanner::plan` → `WorkerCommand::LoadModel`.
9. Display installed model list in `app-tui` with fit annotation (`fits | tight | likely_swap`).

**Deliverable:** User can create a workspace, import txt/md files, see per-stage import status, and switch between locally installed models without restarting.

---

## Phase 3 — Document Q&A (FTS only)

**Goal:** Enable question answering over imported documents using lexical search.

**Steps:**

1. Implement the structure-aware chunker in `retrieval`: heading-first split, paragraph/sentence fallback, overlap, chunk metadata.
2. Implement FTS5 indexing in `retrieval`: insert into `chunk_fts` on document import completion; delete by `chunk_id` before chunk row drop.
3. Implement `ContextBudget` algorithm in `inference`: compute available tokens, pack chunks, pack history (newest-first, dropping oldest), clamp `final_top_k` dynamically.
4. Implement lexical retrieval in `retrieval`: FTS5 query, rank results, apply `max_chunks_per_document` cap.
5. Implement the `document_qa` mode: load from `document_qa.toml`, apply config precedence stack, assemble prompt using the Q&A prompt shape.
6. Wire retrieval into the generation flow in `app`: on `GenerationState → Retrieving`, run retrieval, pass results to `ContextBudget`.
7. Implement source reference persistence: on `GenerationFinished`, insert `message_sources` rows.
8. Display retrieved source citations in `app-tui` (collapsible sources panel).

**Deliverable:** User asks a question; model answers using FTS-retrieved chunks from imported files with source citations.

---

## Phase 4 — Embeddings + Hybrid Retrieval

**Goal:** Upgrade Document Q&A to use both semantic and lexical search.

**Steps:**

1. Implement `embeddings` crate: load `fastembed` model (MiniLM/BGE-small class), embed arbitrary text, return `Vec<f32>`.
2. Implement embedding model registry in `storage`: `embedding_models` table, load/save metadata.
3. Implement background chunk embedding worker in `app`: triggered after FTS indexing completes, emits progress events.
4. Implement `chunk_embeddings` + `chunk_vec_map` transactional writes in `storage` (single transaction, invariant enforced).
5. Implement `VectorIndex` trait wrapping `sqlite-vec` in `retrieval`.
6. Implement query embedding: embed user query via `embeddings` crate at retrieval time.
7. Implement hybrid retrieval in `retrieval`: run vector search and FTS in parallel, merge with weighted scoring formula, deduplicate, apply `max_chunks_per_document` cap.
8. Add retrieval diagnostics panel to `app-tui`: show which chunks were retrieved, their scores, and their sources (toggle-able for debugging).

**Deliverable:** Document Q&A uses both semantic and lexical search with configurable merge weights.

---

## Phase 5 — Model Catalog + Download Manager

**Goal:** Allow users to discover and download models from within the app.

**Steps:**

1. Define the curated model catalog TOML format and ship an initial catalog file.
2. Implement catalog loading in `model-manager`: parse catalog TOML, merge with installed model list.
3. Implement the download worker in `model-manager`: `hf-hub` + `reqwest` for downloads, `sha2` for hash validation, `bytes_downloaded`/`total_bytes` progress emission.
4. Implement the full download state machine (`queued → downloading → complete/failed/cancelled → terminal states`) in `model-manager` and `storage`.
5. Implement user confirmation prompts for retry and restart in `app-tui` (modal dialog).
6. Implement file lifecycle management: partial files to `$XDG_CACHE_HOME/LoreLM/downloads/temp/`, move to `$XDG_DATA_HOME/LoreLM/models/` on completion, delete partial on failure/cancel.
7. Render the model library screen in `app-tui`: `Installed | Downloads | Remote Catalog | Settings` tabs, download progress bars, state machine status display.

**Deliverable:** User can browse the curated model catalog, download models with progress tracking, and retry or cancel downloads with explicit confirmation.

---

## Phase 6 — Modes, Prompts, Exports, Cloning

**Goal:** Complete the full mode system and make conversations exportable and branchable.

**Steps:**

1. Implement all remaining built-in modes: load `summarizer.toml`, `editor_rewriter.toml`, `brainstorming.toml` from `$XDG_CONFIG_HOME/LoreLM/modes/`.
2. Implement Summarizer map-reduce: threshold check, sequential map pass, reduce pass, two distinct system prompts.
3. Implement the mode/prompt editor screen in `app-tui`: mode selector, system prompt editor, restore default, save preset.
4. Implement config precedence stack resolution in `app`: merge global → mode → per-model TOML at generation time.
5. Implement conversation cloning: copy conversation row with `parent_conversation_id`, deep-copy messages.
6. Implement transcript export: serialize conversation + messages to Markdown or plain text, write to user-chosen path.
7. Wire mode switching into the generation flow: reload `ModeDefinition`, update `AppState.active_mode`, reset retrieval config.

**Deliverable:** User can work across all five modes, edit mode prompts, export transcripts, and clone conversations.

---

## Phase 7 — PDF and EPUB

> **Independent of Phase 6.** Can be reordered or worked in parallel.

**Goal:** Extend document ingestion to PDF and EPUB formats.

**Steps:**

1. Implement PDF ingestion in `doc-ingest` (Phase A): text extraction by page via `pdf-extract`, page-number source labels, `page_number` populated on `chunks`.
2. Wire PDF into the import pipeline with the same status stages as text/Markdown.
3. Implement EPUB ingestion in `doc-ingest`: parse spine and TOC via EPUB crate, extract section text, map to `DocumentBlock`s.
4. Add import error reporting improvements: surface per-format error messages via `documents.error_message`.
5. *(Later)* Implement PDF Phase B: heading heuristics from font size/layout metadata.

**Deliverable:** User can import PDFs and EPUBs with text extraction and page/section source references.

---

## Phase 8 — Remote Backend Abstraction

**Goal:** Allow the same TUI to use a remote OpenAI-compatible endpoint in addition to local llama.cpp.

**Steps:**

1. Implement `RemoteBackend` in a new crate or as a second `InferenceBackend` impl: wraps `async-openai` or custom `reqwest` calls.
2. Add remote model config to per-model TOML: endpoint URL, API key (local file reference), model name.
3. Implement backend selection in `app` coordinator: based on `ModelProvider`, route `WorkerCommand` to local or remote backend.
4. Add remote model entries to the model browser.
5. *(Optional)* Implement `axum`-based local API server for exposing LoreLM as an OpenAI-compatible endpoint.

**Deliverable:** Same TUI can use local llama.cpp or a remote OpenAI-compatible endpoint interchangeably.
