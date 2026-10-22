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
- Tests that load a real Whisper model, including the slow/ignored integration tests (`cargo test -- --ignored`).

Report them as skipped in your summary; the user runs them on real hardware before a release. Unit tests and CPU builds without real models (fakes, fixtures) still run as normal.
