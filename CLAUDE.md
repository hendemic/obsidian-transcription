# Local Whisper Transcribe

Obsidian plugin (TypeScript) plus a Rust sidecar that transcribes embedded audio locally with Whisper. See `docs/plan.md`.

## Harness: user-only checks

The development machine may be a VM with no GPU. Check before running any test or build that needs real hardware:

```sh
systemd-detect-virt            # prints the hypervisor in a VM, "none" on bare metal
lspci | grep -iE 'vga|3d'      # a virtual adapter only (e.g. "Device 1234:1111") means no GPU
```

When running in a VM or without a GPU, do not run:

- GPU builds or tests (`--features metal`, `coreml`, `cuda`, `vulkan`).
- Tests that load a real Whisper or paragraph model, including the slow/ignored integration tests (`cargo test -- --ignored`).

Report them as skipped in your summary; the user runs them on real hardware before a release. Unit tests and CPU builds without real models (fakes, fixtures) still run as normal.

## Harness: quality gates

All of these must pass before a branch is done.

Rust (from `sidecar/`):

```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
cargo build
```

Plugin (from `plugin/`):

```sh
npm run typecheck
npm run lint
npm test
npm run build
```

## Testing layout

- Rust unit tests: inline `#[cfg(test)] mod tests` in the source file they cover.
- Rust integration tests: `sidecar/crates/<crate>/tests/`. Slow ones (real model) are `#[ignore]`.
- Test fixtures (e.g. the short WAV): `sidecar/crates/<crate>/tests/fixtures/`.
- Plugin tests: vitest, `plugin/src/**/*.test.ts` next to the code. The `obsidian` module is aliased to `plugin/src/__mocks__/obsidian.ts`; extend that mock as needed.

## Releasing

The sidecar is downloaded from the GitHub release tagged `v<manifest version>`, so plugin and sidecar versions move together.

1. Bump the version in `plugin/manifest.json`, add it to `plugin/versions.json`, and set `[workspace.package] version` in `sidecar/Cargo.toml` (update `Cargo.lock`, and `plugin/package.json` via `npm version` if used).
2. Commit, then tag `vX.Y.Z` and push the tag.
3. `.github/workflows/release.yml` checks tag == manifest == Cargo version, builds the four sidecars (macOS arm64, Linux CUDA, Vulkan, CPU), builds the plugin, and creates a **draft** release with the binaries, `SHA256SUMS`, `main.js`, `manifest.json` and `styles.css`. Review and publish it by hand.

GPU builds (metal/coreml/cuda/vulkan) are only verified by that CI run and on user hardware; they are not built or tested in the dev VM.
