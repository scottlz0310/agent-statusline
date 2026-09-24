# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added
- Core rendering engine (`render`) supporting zero-fork, sub-5ms latency statusline generation.
- Client adapters for Antigravity CLI (`agy`), Claude Code (`claude`), and GitHub Copilot CLI (`copilot`, supporting both `context_window` and legacy `context` schemas).
- In-process Git branch and dirty state detector powered by `gix`.
- Atomic Squirrel Notifier ratelimit status writer (`%LOCALAPPDATA%/SquirrelNotifier/ratelimit-status/<agent>.json`).
- Benchmarking flag (`--bench`) for microsecond-precision latency inspection.
- Comprehensive unit tests covering all adapters, modules, rendering, and atomic file sinks.
- Initial project scaffolding and comprehensive design specifications (`docs/agent-statusline-spec.md`).
- Multi-client architecture design for Antigravity CLI, Claude Code, and GitHub Copilot CLI.
- Zero-downtime self-update design (`agent-statusline update` and background release check).
- Task management and tracking document (`tasks.md`).
