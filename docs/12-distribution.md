# 12 — Distribution

## Build artifacts

`cargo build --release` produces one binary: `mergesmith`. Cross-built for:

- macOS aarch64 (Apple Silicon)
- macOS x86_64
- Linux x86_64
- Linux aarch64

Sprite assets are baked in via `include_bytes!`, so the binary is
self-contained.

## Channels

- **Homebrew tap** — `brew install mergesmith/tap/mergesmith`. Primary
  channel.
- **GitHub Releases** — `.tar.gz` per platform with the binary and a
  `LICENSE` file. Pre-built for `curl | sh` installs.
- **Nix flake** — optional, lightweight.
- **No Mac App Store** — the sandbox is incompatible with spawning git +
  tmux + agent CLIs across arbitrary worktree paths.
- **No Apple Developer Program** in v1 — single Rust binary, no `.app`,
  no Gatekeeper friction. CLI binaries don't trigger the same quarantine
  prompt as `.app` bundles.

## Versioning

SemVer. The SQLite schema version is independent and bumped via
migrations.
