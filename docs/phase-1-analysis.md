# Phase 1 Analysis

Date: 2026-06-24

Scope: unstaged project changes related to Phase 1, reviewed against `docs/progress-tracker.md`, `docs/implementation-plan.md`, `docs/architecture.md`, `docs/phase-0-notes.md`, and `docs/code-standards.md`.

## Summary

The Phase 1 work currently implements most of the application-side scaffolding for minimal local inference, but it does not yet complete the Phase 1 vertical slice.

The main completed work is:

- A backend-neutral inference API and worker command/event protocol.
- A persistent inference worker thread owned by the `app` coordinator.
- A simple `ResourcePlanner` with Linux `/proc/meminfo` RAM estimation and runtime config planning.
- Startup model loading from `config.toml` via `paths.model_path`.
- TUI support for in-progress streaming text, prompt locking during generation, cancellation intent, improved focus handling, and line scrolling.
- Assistant message persistence when a `GenerationFinished` event is received.
- Cleanup items from `docs/phase-0-notes.md`, including use of `ModeDefinition::freeform()` in storage and loading persisted `model_id`.

The remaining critical gap is that `llama-backend` is still a boundary/stub. It verifies that a configured model path exists and records the active model ID, but `generate_stream` returns `InferenceError::Generation("llama.cpp generation is not implemented yet")` for real generation. Because of that, real non-streaming generation, token streaming, full state transitions, and soft-delete cancellation are not complete.

## Files Touched

Unstaged files at review time:

- `Cargo.lock`
- `app/src/coordinator.rs`
- `app/src/logging.rs`
- `app/src/main.rs`
- `crates/app-tui/src/lib.rs`
- `crates/core/src/lib.rs`
- `crates/inference/src/lib.rs`
- `crates/llama-backend/src/lib.rs`
- `crates/model-manager/Cargo.toml`
- `crates/model-manager/src/lib.rs`
- `crates/storage/src/lib.rs`
- `docs/progress-tracker.md`

## High-Level Implementation Details

### Inference API

`crates/inference/src/lib.rs` was expanded from a Phase 0 stub into the central backend-neutral inference contract. It now defines:

- `InferenceBackend`
- `ModelSpec`
- `RuntimeModelConfig`
- `GenerateRequest`
- `WorkerCommand`
- `GenerationEvent`
- `CancellationToken`
- `TokenSink`
- `InferenceError`

`GenerationSummary` and `StopReason` are re-exported from `core`, keeping summary data shared while letting the inference crate own the worker protocol.

This matches the Phase 1 need for a coordinator-to-worker boundary. The API is still larger than the current backend can exercise, but most of it is directly tied to upcoming Phase 1 steps.

### Resource Planning

`crates/model-manager/src/lib.rs` now includes `ResourcePlanner`, with:

- startup RAM estimation from `/proc/meminfo`
- `MemAvailable` parsing
- basic runtime config planning based on whether `model_size * 2 > available_ram`
- unit tests for memory parsing and tight-model planning

`app/src/main.rs` estimates available RAM during startup and passes it into both `Storage::load_app_state` and `Coordinator::new`.

One important implementation detail: `Coordinator::load_configured_model` calls `ResourcePlanner::plan`, then overwrites every planned runtime field with values from `config.inference.defaults`. That means the RAM-aware plan currently has no behavioral effect whenever config defaults are present, which they always are through `AppConfig::default()`.

### Coordinator and Worker Thread

`app/src/coordinator.rs` now owns:

- a persistent `std::sync::mpsc` worker command channel
- a generation event channel
- a dedicated inference worker `JoinHandle`
- active generation state
- active cancellation token
- loaded app config
- available RAM estimate

On startup, the coordinator spawns the worker and attempts to load `paths.model_path` when configured. Prompt submission now persists the user message, creates a `GenerateRequest`, records an `ActiveGeneration`, and sends `WorkerCommand::Generate` to the worker.

Generation events are drained in a polling loop:

- `ModelLoaded` is forwarded to the TUI.
- `TokenDelta` is appended to the active assistant buffer and forwarded.
- `Finished` persists the assistant message and forwards `GenerationFinished`.
- `Cancelled` clears active generation state.
- `Failed` clears active generation state and surfaces an app error.

This replaces the Phase 0 stub assistant response path.

### llama Backend Boundary

`crates/llama-backend/src/lib.rs` now defines `LlamaBackend` and implements `InferenceBackend`.

The current implementation:

- tracks `current_model`
- fails load if the GGUF path does not exist
- unloads by clearing `current_model`
- estimates tokens using whitespace splitting
- returns cancelled summaries if cancellation is already requested
- otherwise returns a generation-not-implemented error

No `llama-cpp-2` dependency or real model/session/token loop has been added yet. This is an intentional boundary, not a working backend.

### TUI Changes

`crates/app-tui/src/lib.rs` now handles a streaming response buffer in `AppState`:

- `TokenDelta` appends to `conversation.streaming_response`.
- streaming text is rendered as an in-progress assistant block.
- `GenerationFinished`, `GenerationCancelled`, and `Error` clear the streaming buffer.
- prompt editing is disabled while generation is active.
- prompt submission sets state to `Preparing`.
- token receipt sets state to `Generating`.
- `Ctrl+C` sends cancellation while generating and sets `Cancelling`.

The Phase 0 TUI notes were mostly addressed:

- `Esc` moves focus out of the prompt.
- `Tab` can insert a tab while the prompt is editable.
- `BackTab` cycles focus backward.
- `Up` and `Down` perform single-line transcript scroll changes.
- newline handling changed from `Alt+Enter` to `Shift+Alt+Enter`, which does not fully match the Phase 0 note's suggested plain `Shift+Enter` behavior.

The state machine remains partial because the backend cannot stream yet and cancellation is only initiated from `Generating`, not `Preparing` or `Retrieving`.

### Storage and Config

`crates/storage/src/lib.rs` now adds:

- `paths.model_path: Option<String>` to `PathConfig`
- `available_ram_bytes` as an input to `load_app_state`
- `streaming_response: None` in initial app state
- `ModeDefinition::freeform()` reuse instead of duplicating the literal
- persisted `model_id` reload support for messages

This aligns with Phase 1's single configured model path and with the Phase 0 cleanup notes.

### Progress Tracker

`docs/progress-tracker.md` was updated to mark the following Phase 1 items complete:

- Apply optimizations and changes listed in `docs/phase-0-notes.md`
- `InferenceBackend` trait and related types defined in `inference`
- `ResourcePlanner` implemented (startup + load-time lifecycle)
- Persistent inference worker thread implemented in `app`
- Message persistence on `GenerationFinished`
- Model path loaded from `config.toml`

The following Phase 1 items remain unchecked:

- `llama-backend` implemented: non-streaming generation
- Streaming generation: `TokenDelta` events to `app-tui`
- `GenerationState` transitions implemented
- Soft-delete cancellation implemented

This is an accurate high-level tracker state. The only nuance is that `ResourcePlanner` exists and is called, but its load-time plan is currently overwritten by config defaults before use.

## Phase 1 Goals Reached

Per `docs/progress-tracker.md`, the project has reached these Phase 1 goals:

1. Phase 0 cleanup items are mostly handled.
   - Freeform mode duplication was removed from storage.
   - Message `model_id` reload was added.
   - Prompt navigation and scrolling were improved.
   - The newline shortcut still differs from the Phase 0 note: the code uses `Shift+Alt+Enter`, not plain `Shift+Enter`.

2. The backend-neutral inference layer exists.
   - The trait, request/response types, worker commands, worker events, and cancellation token are defined.

3. Resource planning exists.
   - Available RAM is estimated at startup.
   - Runtime settings are planned at load time.
   - The estimate is stored in `AppState.available_ram_bytes`.

4. A persistent inference worker thread exists.
   - The coordinator starts one worker thread at startup.
   - The worker receives load/generate/unload/shutdown commands.
   - The worker owns the backend instance.

5. Message persistence on generation finish exists.
   - The coordinator persists an assistant message after a successful finished event.
   - The saved message records `model_id`, `mode_id`, and token count.

6. Model path loading from config exists.
   - `paths.model_path` is accepted in `config.toml`.
   - The coordinator expands `~/`, builds a `ModelSpec`, plans runtime settings, and sends `LoadModel`.

## Phase 1 Goals Not Yet Reached

The Phase 1 deliverable is not complete yet. The user still cannot chat with a local GGUF model from the TUI.

Remaining gaps:

- Real `llama-cpp-2` integration is missing.
- Non-streaming generation is not implemented.
- No backend currently emits token deltas.
- The TUI can render streamed tokens, but that path is not exercised by a real backend.
- `GenerationState` transitions are partial and event-driven rather than fully modeled.
- Cancellation does not soft-delete persisted user/assistant messages.
- Cancellation cannot be requested from all in-flight states.
- Assistant persistence depends on streamed token accumulation; a future non-streaming backend would need to provide final text or emit deltas before `Finished`.

## Validation

Commands run during this analysis:

```bash
cargo fmt --all --check
cargo test --workspace
cargo clippy --all-targets --all-features
```

Results:

- `cargo fmt --all --check`: passed
- `cargo test --workspace`: passed
  - `model-manager`: 2 tests passed
  - `storage`: 1 test passed
  - other crates currently have no tests
- `cargo clippy --all-targets --all-features`: passed

## YAGNI Evaluation

Overall, the Phase 1 implementation is mostly aligned with YAGNI. It builds the minimum architecture needed to replace Phase 0's stub exchange with a real inference pipeline, and it avoids implementing later-phase features such as model scanning, downloads, retrieval, document import, or remote backends.

The best YAGNI-aligned choices are:

- Keeping `llama-backend` isolated behind `InferenceBackend`.
- Using one persistent OS thread for CPU-bound inference instead of introducing broader async machinery.
- Adding `paths.model_path` as a narrow Phase 1 bridge instead of implementing model discovery early.
- Keeping resource planning simple.
- Deferring actual document Q&A, retrieval, model switching, and catalog/download behavior.

There are a few areas where the implementation is ahead of what currently works or could be simplified:

1. `ResourcePlanner` is currently neutralized by config overrides.

   The planner computes a tight/non-tight runtime config, but `Coordinator::load_configured_model` overwrites all fields from `config.inference.defaults`. This creates the appearance of adaptive planning without actual adaptive behavior.

   Recommendation: either let planner output be the default and only override explicitly configured fields, or simplify Phase 1 by using config defaults directly and keeping the planner focused on RAM estimation until per-model settings exist.

2. `model-manager` now depends on `inference`.

   `docs/architecture.md` describes `model-manager` as depending on `core` and `storage`, while this change adds an `inference` dependency for `ModelSpec` and `RuntimeModelConfig`. This may be acceptable for Phase 1, but it is a crate-boundary drift from the architecture document.

   Recommendation: decide whether this dependency is intentional. If yes, update the architecture or decision log. If not, move shared planning input/output types to `core`, or define model-manager-owned planning structs and convert at the app boundary.

3. The generation protocol assumes streaming before real generation exists.

   The protocol and TUI support token deltas, which Phase 1 needs, but the backend cannot emit them yet. This is acceptable scaffolding, but avoid expanding the protocol further until real llama generation drives it.

   Recommendation: next implement the smallest working llama path, even if it starts as non-streaming final text followed by token streaming, before adding more worker states or event variants.

4. Cancellation is represented but not complete.

   The `CancellationToken` is useful, but cancellation does not yet soft-delete messages and the TUI only sends cancellation from `Generating`.

   Recommendation: keep the token, but finish the narrow Phase 1 behavior before adding richer cancellation UX. Specifically, allow cancellation from `Preparing`, `Retrieving`, and `Generating`, then persist cancelled statuses according to the storage model.

5. Coordinator polling is simple but could become noisy later.

   The coordinator uses `try_recv` plus a 20 ms sleep to drain commands and generation events. This is fine for Phase 1, but it is not a long-term event-loop design if more workers are added.

   Recommendation: leave it alone for now unless it causes latency or CPU problems. Revisit when downloads, imports, indexing, and retrieval add more event sources.

## Recommendations

Recommended next steps, in order:

1. Resolve the architecture boundary question around `model-manager -> inference`.
2. Implement real `llama-cpp-2` loading and non-streaming generation in `llama-backend`.
3. Decide how non-streaming generation returns assistant text: emit it as one `TokenDelta`, add final text to `GenerationSummary`, or introduce a final-text event.
4. Add actual token streaming from the backend.
5. Complete `GenerationState` transitions across `Idle`, `Preparing`, `Generating`, `Cancelling`, `Idle`, and `Failed`.
6. Implement soft-delete cancellation for the persisted user/assistant exchange.
7. Add focused tests for config model-path parsing, planner/config merge behavior, coordinator event handling, and cancellation persistence.
