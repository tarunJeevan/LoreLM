# Phase 1 Notes

## `handle_key` in `app-tui`

Current behavior allows `BackTab` and modified `Tab` to cycle UI focus even when `state.ui.focused_panel == Panel::Prompt`. That conflicts with the intended flow: while the prompt is focused, Tab input should remain prompt editing behavior, and the user should press `Esc` to leave the prompt before cycling through other panels.

Recommended fix:

- Keep plain `Tab` insertion in the prompt while the prompt is editable.
- Gate `Tab` and `BackTab` focus cycling behind `state.ui.focused_panel != Panel::Prompt`.
- Preserve `Esc` as the explicit transition between prompt focus and transcript/sidebar/status navigation.

This keeps the primary chat input predictable and matches the TUI layout described in `design.md`.

## Library versions in `Cargo.toml`

A number of 3rd-party libraries/dependencies have newer versions available. Some are minor upgrades while others are major ones. The project should have a proper, up-to-date dependency graph (that doesn't result in version conflicts between each other. E.g., `ratatui` may not support the latest version of `crossterm` just yet) and then pin it to ensure project stability.

Recommended approach:

- Treat dependency updates as a separate stabilization task.
- Audit current versus latest versions with `cargo info <package>`.
- Check compatibility notes before major upgrades, especially for tightly coupled crates such as `ratatui` and `crossterm`.
- Pin the selected compatible versions in `[workspace.dependencies]`.
- Validate with `cargo test --workspace`, `cargo clippy --all-targets --all-features`, and a manual TUI smoke test.

This avoids mixing dependency churn with the still-incomplete `llama-backend` implementation.

## Storage layer

Per `architecture.md`, `storage` is responsible for XDG path resolution, creating default config files, reading TOML, writing TOML, SQLite access, and migrations. However, the public config structs currently defined in `crates/storage/src/lib.rs` are not purely persistence concerns:

- `AppConfig`
- `PathConfig`
- `UiConfig`
- `InferenceConfig`
- `InferenceDefaults`
- `RetrievalConfig`
- `IndexingConfig`
- `ChunkingConfig`

These structs describe application configuration shape that will likely be used outside storage. Keeping them in `storage` forces other crates, especially `app`, to depend on `storage` for shared config types. That weakens the planned boundary where `core` owns shared, serializable domain types and `storage` owns persistence.

Recommended direction:

- Move the serializable config shape types and defaults to `core`.
- Keep `Storage::load_config` and any future save helpers in `storage`.
- Let `storage` deserialize TOML into `core` config types.
- Keep file-system behavior, default-file creation, and TOML I/O out of `core`.

This preserves the invariant that `core` has no I/O while making config types available without coupling callers to persistence.

## Related Architecture Drift

`model-manager` currently depends on `inference` for `ModelSpec` and `RuntimeModelConfig`, but `architecture.md` describes `model-manager` as depending on `core` and `storage`. This is a crate-boundary drift from the planned dependency graph.

Recommended direction:

- Prefer moving shared planning input/output types into `core` if they are domain-level model runtime concepts.
- Alternatively, define model-manager-owned planning structs and convert to inference types at the `app` boundary.

## Priority

1. Fix prompt-focused `Tab` and `BackTab` behavior in `app-tui`.
2. Resolve config type ownership before later config precedence work expands.
3. Audit and pin dependency versions as a separate stabilization task.
4. Keep remaining Phase 1 implementation focused on real `llama-backend` generation, token streaming, full generation-state transitions, and soft-delete cancellation.
