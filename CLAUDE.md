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
- `src/utils.rs` — shared utilities (clipboard, date parsing, terminal width, `parse_fixed_offset`)

## Key design decisions

**Dedup: `messageId:requestId` keep-max-output, global across all files.** Streaming entries have monotonically increasing `output_tokens`; keeping max is order-independent. Same composite key appears in both parent and `subagents/` files — global dedup prevents double-counting. Subagent files are found in both old (`<project>/subagents/<session>_<agent>.jsonl`) and new (`<project>/<session-uuid>/subagents/<agent>.jsonl`) directory structures; `extract_session_id` maps new-structure files to the parent session UUID via the grandparent directory name.

**Pricing: flat rate per-record before grouping.** Each record gets its model's price. Supports `--live-pricing` (runtime fetch via reqwest) and `--pricing-data <path>` (custom file). Bundled pricing embedded via `include_str!`.

**Project paths:** `-home-username-workspace-test` → `/home/username/workspace/test`. Filter with `--project /home/username/workspace/test`.

**Timezone handling:** Uses `chrono` + `chrono-tz` for IANA timezone support, `chrono::FixedOffset` for +HH:MM offsets, `chrono::Local` for system timezone.

**`--cost-diff` requires `--per session`.** Hard error if used with other views.

**Cost-diff totals are like-for-like:** Only matched sessions (those with both SL and LiteLLM costs) are included in totals. Prefix matching uses deterministic longest-match selection.

**sl action view EOF flushing:** `aggregate_ratelimit` skips consecutive records with same (5h%, 7d%) pair. After the main loop, remaining unaccounted cost deltas are flushed to each session's last emitted entry to avoid cost loss.

**sl segment-aware aggregation:** statusline.jsonl has cumulative counters (cost, duration, tokens, lines) that reset to 0 on session resume. Detect resets by comparing consecutive records; sum max-per-segment across segments. For continued sessions (`claude --continue`), cumulative counters carry over from the predecessor without resetting; `aggregate_sessions` subtracts the first record's values as baseline to exclude inherited costs. Window aggregation (`segment_totals_before`) returns the session baseline (not zeros) when no records exist before a time point, ensuring delta computation correctly handles continued sessions.

**sl unified table columns:** All `--per` views (session/project/day/1h/5h/1w) share the same column set: `[Label] | Cost | Duration | API Time | Lines +/- | Sess | 5h% | 1w%`. `--per 1h`/`--per 5h` add `Est 5h Budg` (= cost × 100 / Δ5h%); `--per 1w` adds `Est 1w Budg`. `--per 1h` also adds `5h Resets`. `--per action` includes `Cost` (delta). `5h%` and `1w%` show min–max ranges. Compact mode: `[Label] | Cost | Duration | Sess | 5h%`.

**sl promo adjustment (default on):** Adjusts Est 5h/1w Budget for known 2x usage promo periods (2025-12, 2026-03). Hardcoded UTC intervals in `aggregator.rs`. Adjustment: `adjusted_delta = delta_pct × (1 + promo_overlap_ratio)`. Disable with `--nopromo`.

**sl JSON field naming:** `SlWindowSummary` has separate `est_5h_budget` and `est_1w_budget` fields (serialized as `est5hBudget`/`est1wBudget`). For 1h/5h views, `est_5h_budget` is populated; for 1w views, `est_1w_budget` is populated. The other is `null`.

**`--table auto` behavior:** When writing to file (`--output`/`--filename`), auto selects full table. When printing to terminal, auto selects compact if width < 120.

**Hour key format:** Grouper emits `%Y-%m-%d %H:00` (space separator). Chart `auto_granularity_from_labels` detects hour labels by checking `len > 10 && contains(':')` (matches both space and T separators).

**Types:** `GroupDimension` and `SortOrder` implement `std::str::FromStr` (returns `Result<Self, String>`). Use `"day".parse::<GroupDimension>()` or `GroupDimension::from_str("day")`. CLI supports `-h`/`-V` short flags in addition to `--help`/`--version`.

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
- **clippy** — `cargo clippy -- -W clippy::all` must produce zero warnings
