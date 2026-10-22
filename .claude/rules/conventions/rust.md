---
paths:
  - "**/*.rs"
  - "**/Cargo.toml"
---

# Rust conventions

When writing Rust code, follow the following conventions:
  1. Error handling:
    - anyhow + .context() at app level, custom error types in domain code
    - bail!() for early error returns
    - No unwrap() in production code
  2. Option/Result style:
    - Prefer ? with .context(), .ok_or(), if let, and match — avoid unwrap()
    - Chain with .map(), .filter(), .and_then() over nested if let
  3. Iterators:
    - Prefer iterator chains and functional composition over imperative loops
    - Return impl Iterator to avoid unnecessary allocations
    - Use .find(), .find_map() over manual searching

Avoid `cargo build` unless the user requests it. check work with `cargo check` instead.

# Testing

Applies when writing Rust tests (e.g. as harness:test-agent). The project's `## Harness: testing layout` and `## Harness: quality gates` sections in `CLAUDE.md` take precedence over these defaults.

- Unit tests go in an inline `#[cfg(test)] mod tests` in the source file under test. Integration tests that exercise a crate's public API go in that crate's `tests/` directory.
- Put mocks and helpers at the top of the test module, not in separate files, unless several modules share them.
- Mock at trait boundaries (e.g. an `HttpClient` or `Clock` trait), not internal helper functions.
- If something isn't `pub` enough to test, test it through the public API. Don't widen visibility to make a test possible; flag it instead.
- Skip a test blocked by a known bug with `#[ignore = "blocked by #<bug>: <scenario>"]`.
- For property-based tests, use whichever of `proptest` or `quickcheck` the workspace already depends on.
- Name tests after behaviour: `malformed_header_does_not_panic`, not `test_parse_3`.
- Verify with `cargo test -p <crate>` while iterating, and `cargo test --workspace` before completion.
