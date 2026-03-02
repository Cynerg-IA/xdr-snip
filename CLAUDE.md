# XDR Snip — Project Instructions

> **Layer:** L4 Product — public utility tool
> **Blast radius:** Low — self-contained, no upstream dependencies
> **Approval:** Standard Builder workflow.
> **Universal rules:** global `~/.claude/CLAUDE.md` (applies here — read it)

## What
Windows screenshot tool. Rust workspace: `snip-app` (binary) + `snip-types` (shared types).
Public repo under `Cynerg-IA` org (WAN visible at git.cynerg-ia.com).

## Stack
Rust (Windows-native), workspace with 2 crates. Config: `config.toml` (TOML).

## Build
- `cargo build --release` from workspace root
- `build.ps1` for full release build + packaging
- Version: `crates/snip-app/Cargo.toml` (semver, currently v0.4.6)

## Git
- Primary remote: `gitea`. GitHub suspended.
- Feature branches + PRs. Never push to `main`.

## Code Conventions
- Global rules apply: doc comments, debug logging, OWASP baseline
- Rust: `tracing` for logging, doc comments on all public items
