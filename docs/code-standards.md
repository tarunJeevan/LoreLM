# LoreLM — Code Standards

## 1. General Principles

This is a learning and practice project. Standards should produce idiomatic, readable, and maintainable Rust without being overly prescriptive. When in doubt, prefer clarity over cleverness.

- Prefer explicitness over implicit behavior.
- Prefer simple, flat code over deeply nested abstractions.
- Prefer named types over bare tuples and stringly-typed values.
- Avoid premature optimization. Profile before tuning.

---

## 2. Formatting and Linting

### Formatting

All code must be formatted with `rustfmt` using default settings. Run before every commit:

```bash
cargo fmt --all
```

No custom `rustfmt.toml` overrides. Default settings are sufficient.

### Linting

Clippy with default lints is the baseline. Run before every commit:

```bash
cargo clippy --all-targets --all-features
```

No `#![deny(clippy::all)]` blanket directive — default warning level is sufficient for a learning project. Do not suppress warnings with `#[allow(...)]` without a comment explaining why.

---

## 3. Error Handling

### `thiserror` — internal crates

Use `thiserror` to define typed, structured error enums in all internal library crates: `core`, `storage`, `doc-ingest`, `retrieval`, `embeddings`, `inference`, `llama-backend`, and `model-manager`.

```rust
// Good — typed error in an internal crate
#[derive(Debug, thiserror::Error)]
pub enum StorageError {
    #[error("database error: {0}")]
    Database(#[from] rusqlite::Error),
    #[error("document not found: {id}")]
    NotFound { id: DocumentId },
}
```

Internal errors should be specific enough that a caller can match on variants and handle them meaningfully.

### `anyhow` — application level

Use `anyhow` in `app` and `app-tui` for errors that are logged and surfaced to the user as messages, not handled programmatically.

```rust
// Good — app-level error context
let config = load_config().context("failed to load config.toml")?;
```

Do not use `anyhow` in library crates — callers cannot match on `anyhow::Error` variants.

### Propagation vs. logging

- Propagate errors upward using `?` until they reach a layer that can decide whether to log, display, or recover.
- Log errors at the point of recovery or terminal handling, not at every propagation step.
- Surface user-facing errors as `AppEvent::Error(AppError)` so `app-tui` can display them appropriately.

---

## 4. Async and Threading

### Async runtime

`tokio` is the async runtime. Use `async fn` for:

- Model downloads
- File import and background document indexing
- Embedding workers
- Event channel communication

### Inference worker

The inference worker runs on a **dedicated OS thread** (`std::thread::spawn`), not on the tokio runtime. `llama-cpp-2` is CPU-heavy and blocking — it must not run on the async thread pool.

```rust
// Correct: inference worker on a dedicated thread
std::thread::spawn(move || {
    let mut backend = LlamaBackend::new();
    loop {
        match rx.recv() {
            Ok(WorkerCommand::Generate(req, sink, cancel)) => { /* ... */ }
            Ok(WorkerCommand::Shutdown) => break,
            // ...
        }
    }
});

// Wrong: do not use tokio::spawn or spawn_blocking for the inference loop
tokio::task::spawn_blocking(|| { /* inference loop */ }); // avoid
```

### Channel usage

Use `tokio::sync::mpsc` for async-to-async channels (downloads, indexing progress, event delivery to `app-tui`). Use `std::sync::mpsc` or `crossbeam-channel` for the sync inference worker thread receiving `WorkerCommand`s from the async coordinator.

---

## 5. Documentation

All public-facing items must have doc comments. This is a minimum — more is encouraged.

### Required doc comments

- All `pub trait` definitions, including each method.
- All `pub struct` definitions, including non-obvious fields.
- All `pub enum` definitions, including non-obvious variants.

```rust
/// Embeds arbitrary text and returns a vector of floats.
///
/// This is a pure computation function. It has no knowledge of
/// chunks, documents, or the database.
pub trait Embedder {
    /// Embed a single text string. Returns a vector of length `dimension()`.
    fn embed(&self, text: &str) -> Result<Vec<f32>, EmbeddingError>;

    /// The embedding dimension produced by this model.
    fn dimension(&self) -> usize;
}
```

Private implementation details do not require doc comments but should have inline comments when the logic is non-obvious.

---

## 6. Testing

### Unit tests

Unit tests live in inline `#[cfg(test)]` modules in the same file as the code under test.

```rust
// src/chunker.rs
pub fn chunk(document: &ParsedDocument, config: &ChunkingConfig) -> Vec<DocumentChunk> {
    // ...
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chunk_respects_max_tokens() {
        // ...
    }
}
```

Write unit tests for all non-trivial pure functions. Prioritize:

- Chunking logic (boundary conditions, overlap, minimum chunk size)
- Context budget computation (token counting, priority ordering, clamping)
- Config precedence stack merging
- Retrieval scoring and deduplication logic
- Download state machine transitions

### Integration tests

Integration tests live in a `tests/` directory at the workspace root or within each relevant crate. They test key pipelines end-to-end using real (or realistic stub) data.

Required integration test coverage:

| Pipeline | Location |
|---|---|
| Document import → FTS index → retrieval | `crates/retrieval/tests/` |
| Document import → embedding → vector search → hybrid retrieval | `crates/retrieval/tests/` |
| Inference worker: load model → generate → stream tokens → unload | `crates/llama-backend/tests/` |
| Storage: workspace/document/chunk/message round-trip | `crates/storage/tests/` |
| Config precedence stack: global → mode → per-model merge | `crates/core/tests/` or `app/tests/` |

Integration tests that require a real GGUF model file should be gated with `#[ignore]` and documented:

```rust
#[test]
#[ignore = "requires a local GGUF model at $LORELM_TEST_MODEL_PATH"]
fn test_streaming_inference() {
    // ...
}
```

---

## 7. Codebase Structure Conventions

### Module layout within a crate

Each crate should have a clear `lib.rs` or `mod.rs` that re-exports the public API. Implementation details live in submodules.

```
crates/retrieval/
  src/
    lib.rs          # pub use; crate-level doc comment
    chunker.rs      # Chunker trait + implementation
    fts.rs          # FTS5 query helpers
    vector.rs       # VectorIndex trait + sqlite-vec implementation
    hybrid.rs       # HybridRetriever: merges FTS + vector results
    context.rs      # ContextPacker implementation
    scoring.rs      # merge scoring formula
```

### Naming conventions

Follow standard Rust naming conventions:

| Item | Convention | Example |
|---|---|---|
| Types, traits, enums | `UpperCamelCase` | `DocumentChunk`, `InferenceBackend` |
| Functions, methods, variables | `snake_case` | `pack_chunks`, `available_ram` |
| Constants | `SCREAMING_SNAKE_CASE` | `MAX_CHUNK_TOKENS` |
| Crate names | `kebab-case` | `llama-backend`, `app-tui` |
| Module names | `snake_case` | `mod chunker`, `mod fts` |
| Enum variants | `UpperCamelCase` | `StopReason::EndOfSequence` |

Avoid abbreviations unless they are universally understood in the domain (e.g. `fts`, `gguf`, `ram`, `tui`). Prefer `document_id` over `doc_id`, `embedding_model` over `emb_model`.

### Trait object vs. generics

Prefer trait objects (`Box<dyn Trait>`) for backend-facing traits where the concrete type is determined at runtime and is not performance-critical (e.g. `InferenceBackend`, `VectorIndex`). Prefer generics where the type is known at compile time and performance matters.

### Avoid `unwrap` and `expect` in library code

`unwrap()` and `expect()` are acceptable in tests and in `main()` for truly unrecoverable startup failures. In library crates, always propagate errors with `?`.

```rust
// Bad — panics in library code
let model = models.get(&id).unwrap();

// Good — propagates the error
let model = models.get(&id).ok_or(StorageError::NotFound { id })?;
```

### `unsafe` code

Avoid `unsafe` code unless required by a C FFI boundary (e.g. wrapping `llama-cpp-2`). Any `unsafe` block must have a `// SAFETY:` comment explaining why it is sound.
