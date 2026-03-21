# CLAUDE.md

## Overview

CLI and library: reads Claude Code JSONL from `~/.claude/projects/` and `~/.config/claude/projects/`, deduplicates, calculates costs (LiteLLM pricing), groups by configurable dimensions, outputs table/JSON/markdown/HTML/chart.

Rust port of [cctokens](https://github.com/tomchen/cctokens) (TypeScript).

Pipeline: `load_records → calculate_cost → group_records → format`

`src/` is split into pure library (`lib.rs`, `types.rs`, `parser.rs`, `pricing.rs`, `grouper.rs`, `formatters/`) and CLI (`main.rs`).

## Key design decisions

**Dedup: `messageId:requestId` keep-max-output, global across all files.** Streaming entries have monotonically increasing `output_tokens`; keeping max is order-independent. Same composite key appears in both parent and `subagents/` files — global dedup prevents double-counting.

**Pricing: flat rate per-record before grouping.** Each record gets its model's price. Supports `--live-pricing` (runtime fetch via reqwest) and `--pricing-data <path>` (custom file). Bundled pricing embedded via `include_str!`.

**Project paths:** `-home-username-workspace-test` → `/home/username/workspace/test`. Filter with `--project /home/username/workspace/test`.

**Timezone handling:** Uses `chrono` + `chrono-tz` for IANA timezone support, `chrono::FixedOffset` for +HH:MM offsets, `chrono::Local` for system timezone.

## Commands

```bash
cargo test                  # all tests (169 total)
cargo build --release       # optimized build
cargo run -- --help         # show CLI help
```
