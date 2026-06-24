# LoreLM — Progress Tracker

## Current Status

**Phase:** 1 — Minimal Local Inference
**Goal:** Phase 0 skeleton is implemented. The next work is the minimal local inference vertical slice.

---

## Completion Checklist

### Phase 0 — Skeleton App
- [x] Cargo workspace initialized with all crates stubbed
- [x] Domain IDs and placeholder types defined in `core`
- [x] `AppEvent`, `Command`, `AppState` and sub-state structs defined in `core`
- [x] `ratatui` main loop implemented in `app-tui`
- [x] `app` coordinator skeleton: channel wiring and event dispatch loop
- [x] `storage`: SQLite creation, migrations, XDG path resolution
- [x] Config loading from `config.toml`
- [x] Structured logging wired up
- [x] Stub message persistence and history reload

**Deliverable:** TUI opens, accepts typed prompts, stores stub messages in SQLite, reloads history after restart.

**Note:** Phase 0 creates the regular SQLite schema tables and the FTS5 table. The `sqlite-vec` virtual table is deferred until Phase 4 when the vector extension is introduced.

---

### Phase 1 — Minimal Local Inference
- [x] Apply optimizations and changes listed in `docs/phase-0-notes.md`
- [x] `InferenceBackend` trait and related types defined in `inference`
- [x] `ResourcePlanner` implemented (startup + load-time lifecycle)
- [x] Persistent inference worker thread implemented in `app`
- [ ] `llama-backend` implemented: non-streaming generation
- [ ] Streaming generation: `TokenDelta` events to `app-tui`
- [ ] `GenerationState` transitions implemented
- [ ] Soft-delete cancellation implemented
- [x] Message persistence on `GenerationFinished`
- [x] Model path loaded from `config.toml`

**Deliverable:** User can chat with one local GGUF model from the TUI with streaming output and cancellation. Full vertical slice complete.

---

### Phase 2 — Workspace + Document Import + Model Scanning
- [ ] Workspace creation, selection, and persistence
- [ ] File picker screen implemented
- [ ] `doc-ingest`: plain text and Markdown import pipeline
- [ ] Per-stage import status tracking and display
- [ ] Document sidebar rendered in `app-tui`
- [ ] Model directory scanning implemented
- [ ] Per-model TOML settings: load/save
- [ ] Model switching implemented
- [ ] Installed model list with fit annotation

**Deliverable:** User can create a workspace, import txt/md files, and switch between locally installed models.

---

### Phase 3 — Document Q&A (FTS only)
- [ ] Structure-aware chunker implemented
- [ ] FTS5 indexing implemented
- [ ] `ContextBudget` algorithm implemented with dynamic `final_top_k` clamping
- [ ] Lexical retrieval implemented
- [ ] `document_qa` mode: config loading and prompt assembly
- [ ] Retrieval wired into generation flow
- [ ] Source reference persistence
- [ ] Source citations displayed in `app-tui`

**Deliverable:** User asks a question; model answers using FTS-retrieved chunks with source citations.

---

### Phase 4 — Embeddings + Hybrid Retrieval
- [ ] `embeddings` crate implemented (`fastembed`, `ort`)
- [ ] Embedding model registry in `storage`
- [ ] Background chunk embedding worker
- [ ] Transactional `chunk_embeddings` + `chunk_vec_map` writes
- [ ] `VectorIndex` trait wrapping `sqlite-vec`
- [ ] Query embedding at retrieval time
- [ ] Hybrid retrieval: vector + FTS merge with weighted scoring
- [ ] Retrieval diagnostics panel in `app-tui`

**Deliverable:** Document Q&A uses both semantic and lexical search.

---

### Phase 5 — Model Catalog + Download Manager
- [ ] Curated model catalog TOML format defined and initial catalog shipped
- [ ] Catalog loading in `model-manager`
- [ ] Download worker implemented (hf-hub, reqwest, sha2)
- [ ] Full download state machine implemented
- [ ] User confirmation prompts for retry/restart
- [ ] File lifecycle management (temp → models dir)
- [ ] Model library screen rendered in `app-tui`

**Deliverable:** User can browse catalog, download models, and retry or cancel with confirmation.

---

### Phase 6 — Modes, Prompts, Exports, Cloning
- [ ] All built-in modes loaded from TOML
- [ ] Summarizer map-reduce implemented
- [ ] Mode/prompt editor screen implemented
- [ ] Config precedence stack resolution implemented
- [ ] Conversation cloning implemented
- [ ] Transcript export implemented (Markdown + plain text)
- [ ] Mode switching wired into generation flow

**Deliverable:** User can work across all modes, export transcripts, and clone conversations.

---

### Phase 7 — PDF and EPUB *(independent of Phase 6)*
- [ ] PDF ingestion Phase A: text by page, `pdf-extract`
- [ ] PDF wired into import pipeline with status stages
- [ ] EPUB ingestion: spine/TOC parsing, section extraction
- [ ] Import error reporting improvements

**Deliverable:** User can import PDFs and EPUBs with source references.

---

### Phase 8 — Remote Backend Abstraction
- [ ] `RemoteBackend` implementing `InferenceBackend`
- [ ] Remote model config in per-model TOML
- [ ] Backend selection in `app` coordinator
- [ ] Remote model entries in model browser
- [ ] *(Optional)* Local `axum` API server

**Deliverable:** Same TUI works with local llama.cpp or a remote OpenAI-compatible endpoint.

---

## Architectural Decision Log

No decisions recorded yet. This section will track decisions made during implementation that contradict or alter what was previously specified in the architecture, design, or implementation plan documents. Each entry should include:

- **Date**
- **Decision:** What was decided
- **Replaces:** What it contradicts or changes
- **Reason:** Why the original decision was altered

### Decision 1 - Update Workspace Rust Edition

- **Date:** 06-21-2026
- **Decision:** Updated the workspace rust edition from 2021 to 2024.
- **Replaces:** It introduces 1 or 2 warnings about certain values being dropped earlier or later due to the edition change.
- **Reason:** There was no reason for sticking to the 2021 edition. Updating to the latest edition is more forward-facing as many third-party crates do the same, potentially introducing instabilities from dependencies.

---

*Last updated: Phase 0 complete*
