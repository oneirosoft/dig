# AGENTS.md

This file documents repo-specific guidance for future coding sessions in this repository.

## Goals

- Keep `dig` maintainable as it grows into a stacked-PR workflow tool.
- Prefer designs that scale to more commands, richer TUI interactions, and more complex branch graph behavior.
- Avoid quick fixes that introduce brittle data models or tightly coupled command logic.

## Testing Expectations

- When adding or changing functionality, add or update tests in the same change whenever practical.
- Prefer testing at the right boundary:
  - unit tests for pure data modeling, parsing, rendering, and graph logic
  - integration or smoke tests for Git-backed workflows and command behavior
- If behavior changes, existing tests should be updated to reflect the intended new behavior rather than left stale.
- `cargo check` and `cargo test` should be treated as the default verification baseline.

## Smoke Testing

Smoke tests are expected for Git workflow changes, especially for commands that mutate repository state.

Typical smoke-test pattern:

1. Create a temporary repository with `git init -b main`.
2. Configure local Git identity in the temp repo.
3. Disable local commit signing in the temp repo when needed:
   - `git config commit.gpgSign false`
4. Create minimal commits needed to produce realistic branch divergence.
5. Run the built `dig` binary against the temp repo.
6. Verify both:
   - visible command output
   - persisted state under `<git-dir>/dig/`

When validating branching behavior, confirm both:

- the actual Git branch base via commit OIDs
- the stored dig metadata in `state.json`

When validating tree or lineage output, prefer constructing a temp repo with:

- one linear stack
- one sibling branch from the same parent
- one separate stack from trunk

## Data Modeling

- Prefer unions/enums over nullable fields or magic string sentinels where possible.
- Model domain state explicitly and make invalid states hard to represent.
- Avoid using `null` when a tagged union or enum variant expresses intent better.
- Avoid stringly typed control flow when a dedicated type can represent the domain more clearly.

Examples of preferred style:

- `ParentRef::Trunk`
- `ParentRef::Branch { node_id }`

Examples to avoid unless there is a strong reason:

- `parent_id = null`
- `parent_id = "trunk"`

## Architecture

- Keep CLI parsing separate from core domain logic.
- Keep rendering and presentation logic separate from core graph/query logic.
- Prefer small command adapters in `src/cli/` that translate into core operations.
- Prefer core modules that return structured outcomes rather than preformatted strings.
- Store-related concerns should stay under `src/core/store/`.

## Scalability and Extensibility

Write code so future features can be added without large rewrites.

Prefer:

- explicit domain types
- structured outcomes
- composable helper functions
- localized side effects
- testable pure functions where possible

Be cautious about:

- tightly coupling Git execution to presentation
- duplicating branch graph logic in multiple layers
- encoding future assumptions into one-off command code

## Design Principles

- Follow SOLID principles where they improve clarity and separation of concerns.
- Prefer functional principles where practical:
  - pure functions for transforms and renderers
  - data-in/data-out helpers
  - minimized shared mutable state
- Keep imperative logic near the boundaries:
  - filesystem
  - subprocess execution
  - terminal output

## Command and Output Conventions

- Command files should remain under `src/cli/`.
- Shared command-specific helpers may live in subdirectories when needed.
- Keep output formatting consistent with the repo’s existing conventions:
  - simple lineage view for focused branch output
  - shared-root tree view for `dig tree`
  - use shared UI markers and palette definitions

## Verification Notes

For formatting, use `rustfmt` from the Rust toolchain managed by `rustup`, not a Homebrew-installed formatter. CI installs `rustfmt` on the stable toolchain and runs `cargo fmt --all --check`, so local verification should use the same toolchain, for example:

- `rustup component add rustfmt`
- `rustup run stable cargo fmt --all`

Before closing a session that changes behavior:

- run `cargo check`
- run `cargo test`
- run a focused smoke test if Git behavior, branching, commit flow, or tree rendering changed

If smoke testing is not possible, explicitly note that in the final handoff.
