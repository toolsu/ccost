# CLAUDE.md

## Overview

CLI and library: reads Claude Code JSONL from `~/.claude/projects/` and `~/.config/claude/projects/`, deduplicates, calculates costs (LiteLLM pricing), groups by configurable dimensions, outputs table/JSON/markdown/HTML/chart.

Also includes `ccost sl` subcommand: reads `~/.claude/statusline.jsonl` for rate limit tracking, session summaries, 5h budget estimation, and cost cross-comparison.

Rust port of [ccost](https://github.com/toolsu/ccost) (TypeScript).

## Architecture

Main pipeline: `load_records → calculate_cost → group_records → format`

sl pipeline: `load_sl_records → filter → aggregate (segment-aware) → format`

`src/` is split into pure library (`lib.rs`, `types.rs`, `parser.rs`, `pricing.rs`, `grouper.rs`, `formatters/`) and CLI (`main.rs`).

- `src/sl/` — statusline analysis module: `types.rs`, `parser.rs`, `aggregator.rs`, `formatter.rs`
- `src/utils.rs` — shared utilities (clipboard, date parsing, terminal width)

## Key design decisions

**Dedup: `messageId:requestId` keep-max-output, global across all files.** Streaming entries have monotonically increasing `output_tokens`; keeping max is order-independent. Same composite key appears in both parent and `subagents/` files — global dedup prevents double-counting.

**Pricing: flat rate per-record before grouping.** Each record gets its model's price. Supports `--live-pricing` (runtime fetch via reqwest) and `--pricing-data <path>` (custom file). Bundled pricing embedded via `include_str!`.

**Project paths:** `-home-username-workspace-test` → `/home/username/workspace/test`. Filter with `--project /home/username/workspace/test`.

**Timezone handling:** Uses `chrono` + `chrono-tz` for IANA timezone support, `chrono::FixedOffset` for +HH:MM offsets, `chrono::Local` for system timezone.

**sl segment-aware aggregation:** statusline.jsonl has cumulative counters (cost, duration, tokens, lines) that reset to 0 on session resume. Detect resets by comparing consecutive records; sum max-per-segment across segments.

**sl unified table columns:** All `--per` views (session/project/day/1h/5h/1w) share the same column set: `[Label] | Cost | Duration | API Time | Lines +/- | Sess | 5h% | 1w%`. `--per 1h`/`--per 5h` add `Est 5h Budg` (= cost × 100 / Δ5h%); `--per 1w` adds `Est 1w Budg`. `--per 1h` also adds `5h Resets`. `--per action` includes `Cost` (delta). `5h%` and `1w%` show min–max ranges. Compact mode: `[Label] | Cost | Duration | Sess | 5h%`.

**sl promo adjustment (default on):** Adjusts Est Budget for known 2x usage promo periods (2025-12, 2026-03). Hardcoded UTC intervals in `aggregator.rs`. Adjustment: `adjusted_delta = delta_pct × (1 + promo_overlap_ratio)`. Disable with `--nopromo`.

**`--table auto` behavior:** When writing to file (`--output`/`--filename`), auto selects full table. When printing to terminal, auto selects compact if width < 120.

## Commands

```bash
cargo test                  # all tests
cargo build --release       # optimized build
cargo run -- --help         # show CLI help
cargo run -- sl --help      # show sl subcommand help
```

## Maintenance rules

**Every code change must also update, if applicable:**
- **Tests** — add or update tests to cover the change
- **README.md** — update documentation, CLI reference, examples
- **CLAUDE.md** — update this file if architecture, design decisions, or conventions change
