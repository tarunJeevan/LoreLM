# Notes on phase 0

## Notes on TUI loop in app_tui

### handle_key() function

- Current key combination for entering a newline without entering is Alt+Enter. Changing this to Shift+Enter would be more ergonomic and in-line with other chatbots and LLM apps.
- Current key combination for leaving the prompt window and set focus to the wider app is the Tab key. Changing this to Esc is more ergonomic and allows the Tab key to be used for adding a '\t' to the prompt if needed.
- Scrolling behavior with Page Up and Page Down exists but line navigation with Arrow keys is not explicitly implemented (though it might be supported out of the box in some way) so it requires checking.

## Notes on duplication in storage crate

- `Storage::load_app_state` duplicates the `ModeDefinition::freeform` literal instead of calling the existing constructor defined in the `core` crate.

## YAGNI (You Aren't Gonna Need It) Refactor Suggestions

These suggestions apply the YAGNI principle: keep the implementation only as
large as the next milestone needs.

1. Defer broader config plumbing until it affects behavior.

   `AppConfig` is already loaded, but most fields are for later phases. Avoid
   building a full config precedence or runtime settings system until Phase 1 or
   Phase 6 needs it. For now, either keep the unused parameter explicitly named
   `_config` or only thread through the specific Phase 1 model path when that
   work starts.

2. Avoid growing placeholder crates before their phase begins.

   The stub crates are enough to prove workspace structure. Do not add traits,
   DTOs, or module trees to `doc-ingest`, `retrieval`, `embeddings`,
   `model-manager`, or `llama-backend` until the implementation plan reaches the
   phase that needs them. The exception is `inference`, which is first in Phase 1
   and should get the minimal backend-neutral API required by the vertical slice.

3. Trim or tolerate unused binary dependencies intentionally.

   The `app` crate currently depends on all internal crates to match the planned
   coordinator role. If compile time or warnings become a problem, remove unused
   internal dependencies and add them back when wiring begins. If not, leaving
   them is acceptable because it documents intended ownership.

4. Keep TUI placeholders passive until real workflows exist.

   The sidebar, document panel state, model panel state, modal stack, and screen
   enum are useful scaffolding. Avoid implementing navigation and state mutation
   for file import, model library, settings, or mode editor screens until the
   relevant phase adds actual data and behavior.

5. Implement only the next slice of inference, not the full v1 abstraction.

   Phase 1 needs one local GGUF model, a persistent worker thread, streaming
   token events, cancellation, and assistant persistence. Avoid adding remote
   backend shape, catalog metadata, prompt editor behavior, or hybrid retrieval
   policy logic during that work.

6. Add tests around behavior, not placeholders.

   The existing storage test is useful because it exercises real Phase 0
   behavior. Continue that pattern: add tests for sequence numbering,
   cancellation persistence, worker event transitions, config parsing, or
   message reload when those behaviors exist. Avoid tests that only assert
   placeholder `NotImplemented` errors.

7. Fix small duplication before it spreads.

   The best low-cost cleanup is consolidating freeform mode construction and
   preserving `model_id` in `load_messages` once generation starts assigning it.
   Both changes keep the code simpler without committing to later-phase designs.
