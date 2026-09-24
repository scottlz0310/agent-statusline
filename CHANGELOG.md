# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added
- Self-update subcommand (`agent-statusline update`) with `--check` and `--force` flags, supporting zero-downtime in-process binary replacement with atomic staging (`.new`), `.old` backup, and rollback protection on failure.
- 24-hour asynchronous background update check during `render` execution with zero latency impact (sub-millisecond cache timestamp inspection and detached background process spawn).
- `cargo-dist` release pipeline configuration (`dist-workspace.toml`, `Cargo.toml` profile.dist, `.github/workflows/release.yml`) generating cross-compilation release builds, checksums, and shell/PowerShell installers for Windows (`x86_64-pc-windows-msvc`), Linux musl (`x86_64-unknown-linux-musl`), and ARM Linux musl (`aarch64-unknown-linux-musl`).
- Windows standalone PowerShell installer script (`scripts/agent-statusline-installer.ps1`) targeting `%LOCALAPPDATA%\Programs\agent-statusline` and configuring User `PATH`.
- Lefthook Git hooks configuration (`lefthook.yml`) executing `rustfmt`, `clippy`, code-behind size ratchet, and tests locally on pre-commit and pre-push, with installation and hook registration documented in `README.md`.
- Renovate configuration (`renovate.json`) integrating shared presets from `scottlz0310/renovate-config` for Rust, PowerShell, Lefthook, automerge, schedule, and security.
- Codecov coverage measurement pipeline in CI using `cargo-llvm-cov` with initial `codecov.yml` in Informative mode (#9).
- Code-behind line count ratchet guard script (`scripts/check-code-behind-size.ps1`) to strictly prevent logic creep in untested entrypoints (#9).
- Starship-like TOML configuration engine (`config.toml`) supporting declarative statusline layouts and module customization (`src/engine/config.rs`).
- Inline style syntax parser (`[text](style)`) translating styles, modifiers, and foreground/background colors into ANSI escape sequences (`src/engine/style.rs`).
- Dynamic template variable expansion engine for all statusline modules with zero-configuration fallback (`src/engine/formatter.rs`).
- `--config <PATH>` CLI option on `render` subcommand and `AGENT_STATUSLINE_CONFIG` environment variable resolution.
- Automated client installer (`install`), uninstaller (`uninstall`), and configuration diagnostics (`status`) for Antigravity, Claude Code, and GitHub Copilot CLI.

### Fixed
- Expand environment variables (`%VAR%`, `$VAR`, `${VAR}`) in `output_dir` for custom Squirrel Notifier ratelimit output.
- Eliminate redundant in-process Git status lookups and evaluate modules conditionally in template renderer.
- Prevent false positives during uninstall and diagnostics when custom wrapper commands include `agent-statusline` in their name by strictly verifying command basename and arguments.
- Safe JSON configuration patcher preserving existing settings, atomic file replacement, automated backup creation (`.bak`), and JSONC comment preservation.
- Dry-run mode (`--dry-run`) with formatted JSON preview for safe configuration inspection.
- Core rendering engine (`render`) supporting zero-fork, sub-5ms latency statusline generation.
- Client adapters for Antigravity CLI (`agy`), Claude Code (`claude`), and GitHub Copilot CLI (`copilot`, supporting both `context_window` and legacy `context` schemas with standalone context token accounting).
- In-process Git branch and dirty state detector powered by `gix`.
- Atomic Squirrel Notifier ratelimit status writer (`%LOCALAPPDATA%/SquirrelNotifier/ratelimit-status/<agent>.json`).
- Benchmarking flag (`--bench`) for microsecond-precision latency inspection.
- Comprehensive unit tests covering all adapters, modules, rendering, and atomic file sinks.
- Initial project scaffolding and comprehensive design specifications (`docs/agent-statusline-spec.md`).
- Multi-client architecture design for Antigravity CLI, Claude Code, and GitHub Copilot CLI.
- Zero-downtime self-update design (`agent-statusline update` and background release check).
- Task management and tracking document (`tasks.md`).
