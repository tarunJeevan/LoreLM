# LoreLM Agent Instructions

These instructions guide coding agents working on LoreLM. Treat this file as the always-loaded routing layer. The supporting documents located under the `docs/` directory are the authoritative source for product behavior, architecture, implementation sequencing, progress tracking, and code style. 

## Supporting Documents

Read the relevant supporting document in the `docs/` directory before changing code, tests, docs, or project structure.

| Document | Use it for |
|---|---|
| `design.md` | Product goals, v1 scope, non-goals, user flows, mode behavior, document ingestion behavior, model-management behavior, TUI layout, keybindings, and configuration shape. |
| `architecture.md` | Cargo workspace layout, crate dependency boundaries, module responsibilities, database schema, config precedence, core traits, generation state machine, and invariants. |
| `implementation-plan.md` | Phase order, milestone sequencing, per-phase steps, and phase deliverables. |
| `progress-tracker.md` | Current phase, completed work, next unchecked tasks, and architectural decision log. |
| `code-standards.md` | Rust style, formatting, linting, error handling, async/threading rules, documentation requirements, tests, module layout, naming, `unwrap`/`expect`, and `unsafe`. |

## Default Workflow

Before starting implementation work:

1. Read `progress-tracker.md` to determine the current phase and relevant unchecked work.
2. Read `implementation-plan.md` for phase boundaries and deliverables.
3. Read the supporting docs needed for that task.
4. State any uncertainties regarding the task. Do not assume or hide confusion.
4. List the files/crates expected to change before making broad edits.
5. Make the smallest coherent change that satisfies the phase deliverable. Do not perform large opportunistic refactors during feature work. Keep changes phase-aligned and easy to review.
6. Add or update tests for behavior changes.

After completing implementation work:

1. Run the narrowest relevant validation command, then broader checks when appropriate.
2. Report a summary of what was changed, what was tested, and any follow-up work left unchecked in `progress-tracker.md`.
3. When a task is completed, update `progress-tracker.md` by checking off the corresponding item.
4. If a decision has been made by the user that changes or contradicts an existing supporting document, add an entry to the Architectural Decision Log in `progress-tracker.md`.

## Source of Truth and Conflict Resolution

When documents overlap, resolve conflicts in this order:

1. User's latest explicit instruction.
2. The Architectural Decision Log in `progress-tracker.md`, if it records an accepted change.
3. `architecture.md` for crate boundaries, invariants, data model, and state machines.
4. `design.md` for product behavior, v1 scope, UX, modes, configuration examples, and non-goals.
5. `implementation-plan.md` for sequencing.
6. `code-standards.md` for style and quality practices.
7. This `AGENTS.md` routing file.

If a requested change would violate in invariant or contradict a supporting document, pause, explain the conflict, and ask the user for a final decision before changing code. Prefer an explicit decision-log update over silently drifting from the plan.

## Standing Guardrails

- Do not duplicate detailed phase guidance, settings tables, schemas, tests, or implementation rules in this file; link back to the supporting documents instead.
- Do not implement later-phase features unless the user explicitly asks or a small placeholder is needed to avoid architectural rework.
- Do not broaden v1 scope beyond `design.md` unless the user explicitly changes scope.
- Preserve the crate boundaries and invariants in `architecture.md`.
- Follow `code-standards.md` for formatting, linting, errors, docs, tests, async, threading, and unsafe code.
- Do not modify `.codex/config.toml` unless the user explicitly asks; it contains project-specific Codex settings.
- Avoid opportunistic refactors during feature work.
- Prefer updating code to match the supporting docs. If the docs are wrong or stale, ask the user for a final decision.
- Record all user-made architectural decisions during implementation in the Architectural Decision Log in `progress=tracker.md`.

## Completion Response Expectations

When reporting back, include:

- A brief summary of what changed.
- Files or crates touched.
- Validation commands run and results.
- Whether `progress-tracker.md` was updated.
- Any deferred or follow-up work.
