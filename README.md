# ccost <img src="logo.svg" alt="ccost" width="35" />

Analyze Claude Code token usage and cost from your local conversation history.

Rust port of [ccost](https://github.com/toolsu/ccost) (TypeScript).

Reads JSONL files from `~/.claude/projects/` and `~/.config/claude/projects/`, deduplicates streaming entries, calculates costs using LiteLLM pricing, groups by configurable dimensions, and outputs as table, JSON, Markdown, HTML, CSV, TSV, or braille chart.

Also includes `ccost sl` — a subcommand that analyzes `~/.claude/statusline.jsonl` for **rate limit tracking**, session summaries, 5-hour budget estimation, and cost cross-comparison.

## Install

```bash
cargo install --path .
```

Or run directly without installing:

```bash
cargo run --release -- [options]
```

## Quick Start

```bash
# Show token usage grouped by day then by model (default)
ccost

# Show usage for a specific date range
ccost --from 2026-03-01 --to 2026-03-31

# Group by model only
ccost --per model

# Group by model with 2-decimal cost breakdown
ccost --per model --cost decimal

# Group by project for March 2026
ccost --per project --from 2026-03-01 --to 2026-03-31

# Show cost chart over time
ccost --chart cost

# Show token usage chart
ccost --chart token --from 2026-03-01

# Statusline analysis: session summaries (default)
ccost sl

# Rate limit timeline (per action)
ccost sl --per action

# 5-hour budget estimation
ccost sl --per 5h --cost decimal

# Rate limit chart over time
ccost sl --chart 5h
```

## CLI Reference

### Grouping

| Flag | Description |
|------|-------------|
| `--per <dim>` | Group by dimension: `day`, `hour`, `month`, `session`, `project`, `model`. Can be specified up to 2 times for two-level grouping. Default: `--per day --per model`. |
| `--order <order>` | Sort order for group labels: `asc` (default) or `desc`. |

Two-level grouping produces a parent-child hierarchy. For example, `--per day --per model` shows each day as a parent row with per-model children underneath.

### Date Filtering

| Flag | Description |
|------|-------------|
| `--from <date>` | Start date (inclusive). Format: `YYYY-MM-DD` or `YYYY-MM-DDTHH:MM:SS`. |
| `--to <date>` | End date (inclusive). Same formats as `--from`. |
| `--5hfrom <datetime>` | 5-hour window starting at the given time. Shortcut for `--from T --to T+5h`. |
| `--5hto <datetime>` | 5-hour window ending at the given time. Shortcut for `--from T-5h --to T`. |
| `--1wfrom <datetime>` | 1-week window starting at the given time. Shortcut for `--from T --to T+7d`. |
| `--1wto <datetime>` | 1-week window ending at the given time. Shortcut for `--from T-7d --to T`. |

Constraints:
- `--5hfrom` and `--5hto` cannot be used together.
- `--1wfrom` and `--1wto` cannot be used together.
- `--5h*` and `--1w*` cannot be used together.
- `--5h*`/`--1w*` cannot be combined with `--from`/`--to`.

### Record Filtering

| Flag | Description |
|------|-------------|
| `--project <name>` | Filter by project path (case-insensitive substring match). Projects are decoded from directory names (e.g. `-home-user-project` becomes `/home/user/project`). |
| `--model <name>` | Filter by model name (case-insensitive substring match). Example: `--model opus` matches `claude-opus-4-6-20250618`. |
| `--session <id>` | Filter by session ID (case-insensitive substring match). Session IDs come from JSONL filenames. |

### Timezone

| Flag | Description |
|------|-------------|
| `--tz <tz>` | Timezone for date formatting and grouping. Accepts: `UTC`, fixed offset `+HH:MM` or `-HH:MM`, or IANA name (e.g. `Asia/Shanghai`, `America/New_York`). Default: system local timezone. |

### Cost Display

| Flag | Description |
|------|-------------|
| `--cost <mode>` | `true` (default): integer dollars `($1)`. `decimal`: 2 decimal places `($1.23)`. `false`: hide costs entirely. |

### Output

| Flag | Description |
|------|-------------|
| `--output <fmt>` | Output format: `json`, `markdown`, `html`, `txt`, `csv`, `tsv`. Writes to file (default name: `ccost.<ext>`). |
| `--filename <path>` | Custom output filename. Works with or without `--output`. |
| `--table <mode>` | Table display: `auto` (default), `full`, `compact`. `auto` uses full when writing to file (`--output`/`--filename`), compact when terminal width < 120. `compact` hides the `In`, `Cache Cr`, `Cache Rd` columns, showing only `In Total`, `Out`, `Total`. |
| `--copy <fmt>` | Copy formatted output to clipboard: `json`, `markdown`, `html`, `txt`, `csv`, `tsv`. Can be used alongside all other flags. Uses native clipboard tools (`pbcopy`/`clip`/`xclip`/`xsel`/`wl-copy`) with OSC 52 terminal escape sequence as fallback. |

When no `--output` is specified, output goes to stdout as a colored table. When `--output` or `--filename` is specified, output is written to a file. `--copy` is independent of the main output — it can be used alone or together with `--output`/`--filename`.

### Chart

| Flag | Description |
|------|-------------|
| `--chart <mode>` | Show a braille line chart in the terminal: `cost` or `token`. |

Chart behavior:
- Granularity is auto-selected based on date range: hour (<=2 days), day (<=90 days), month (>90 days).
- `--per`, `--cost`, `--order` are ignored in chart mode.
- Only `--output txt` is allowed with `--chart` (other formats are rejected).
- All filter flags (`--from`, `--to`, `--project`, `--model`, `--session`, `--tz`) are respected.

### Pricing

| Flag | Description |
|------|-------------|
| `--live-pricing` | Fetch latest pricing from LiteLLM's GitHub at runtime (requires network). |
| `--pricing-data <path>` | Use a custom pricing JSON file instead of the bundled one. |

`--live-pricing` and `--pricing-data` cannot be used together.

### Other

| Flag | Description |
|------|-------------|
| `--claude-dir <dir>` | Override the default Claude config directory. Default locations: `~/.claude` and `~/.config/claude`. |
| `--help` | Show help message. |
| `--version` | Show version number. |

## Statusline Data Analysis (`ccost sl`)

Analyzes `~/.claude/statusline.jsonl` — a file generated by a Claude Code statusline hook that captures per-action snapshots including rate limits, cost, duration, and context window usage.

### Setup

If you haven't set up a custom status line, do it now. In `~/.claude/settings.json`, add:

```json
  "statusLine": {
    "type": "command",
    "command": "~/.claude/statusline.sh"
  }
```

Create `~/.claude/statusline.sh` with the following content:

```bash
#!/bin/bash
input=$(cat)
echo "{\"ts\":$(date +%s),\"data\":$input}" >> ~/.claude/statusline.jsonl
```

This appends one JSON line per Claude Code action to `~/.claude/statusline.jsonl`.

Mark the script as executable:

```sh
chmod +x ~/.claude/statusline.sh
```

If you are already using a `~/.claude/statusline.sh`, just add `echo "{\"ts\":$(date +%s),\"data\":$input}" >> ~/.claude/statusline.jsonl` at the end.

### View Modes

| Usage | Description |
|-------|-------------|
| `ccost sl` | 5-hour windows (default) |
| `ccost sl --per action` | Rate limit timeline — shows cost, 5h% and 1w% changes per action |
| `ccost sl --per session` | Group by session |
| `ccost sl --per project` | Group by project |
| `ccost sl --per day` | Group by day |
| `ccost sl --per 1h` | Group by 1-hour windows within 5h windows (adds Est 5h Budg + 5h Resets columns) |
| `ccost sl --per 5h` | Group by 5-hour rate-limit window (adds Est 5h Budg column) |
| `ccost sl --per 1w` | Group by 1-week rate-limit window (adds Est 1w Budg column) |
| `ccost sl --chart 5h` | 5-hour rate limit percentage chart |
| `ccost sl --chart 1w` | 1-week rate limit percentage chart |
| `ccost sl --chart cost` | Cumulative cost chart |

All `--per` views share the same columns: `Cost`, `Duration`, `API Time`, `Lines +/-`, `Sess`, `5h%`, `1w%`. Only the first column (the grouping label) changes. `--per 1h` / `--per 5h` add `Est 5h Budg`; `--per 1w` adds `Est 1w Budg`; `--per 1h` also adds `5h Resets`. The `5h%` and `1w%` columns show min–max ranges (e.g. `1–29%`).

### Statusline-Specific Flags

| Flag | Description |
|------|-------------|
| `--file <path>` | Statusline file path. Default: `~/.claude/statusline.jsonl`. |
| `--nopromo` | Disable promo adjustment for Est Budget (default: adjusts for known 2x promo periods). See [Promo Adjustment](#promo-adjustment). |
| `--cost-diff` | Compare statusline self-reported cost with LiteLLM-calculated cost. Only with `--per session`. |

All shared flags (`--from`, `--to`, `--tz`, `--session`, `--project`, `--model`, `--cost`, `--output`, `--filename`, `--copy`, `--order`, `--table`, `--5hfrom`, `--5hto`, `--1wfrom`, `--1wto`) work with `ccost sl`.

### Segment-Aware Aggregation

When a Claude Code session is closed and resumed, cumulative counters (cost, duration, tokens, lines) reset to zero. `ccost sl` detects these resets and correctly sums across segments:

```
Session ef2fa622: 3 segments
  Segment 1: $0.68 (730s)
  Segment 2: $1.87 (4761s)
  Segment 3: $0.00 (0s)
  Total:     $2.55 (5491s)
```

Rate limits are account-level and do NOT reset with sessions.

### Budget Estimation

The `--per 1h`, `--per 5h`, and `--per 1w` views estimate total budget from actual cost and rate limit percentage delta:

```
Est 5h Budg = Cost in Window × 100 / Δ5h%
Est 1w Budg = Cost in Window × 100 / Δ1w%
```

Where `Δ5h% = max 5h% − min 5h%` within that window. This uses the delta (how much rate limit was consumed during the window) rather than the absolute peak, giving a more accurate estimate.

**Limitation:** `ccost sl` only has access to Claude Code cost data. Usage from other Claude products (claude.ai web, Claude Desktop/Mobile App, Claude Code web, etc.) also contributes to the rate limit percentage but is not reflected in the cost. This causes Est Budget to be **underestimated**: the numerator (CC cost only) is too small while the denominator (Δ5h% from all products) is too large. The degree of underestimation is proportional to how much non-Claude-Code usage occurred during the window.

### Promo Adjustment

By default, `ccost sl` adjusts budget estimates for known double-usage (2x) promotional periods. During these promos, the rate limit budget is effectively doubled, so the same dollar amount consumes half the rate limit percentage. Use `--nopromo` to disable this adjustment:

```sh
ccost sl --per 5h --nopromo
```

The following promo periods are built in (all UTC):
- **2025-12-24 to 2025-12-31**: Full week double usage
- **2026-03-10 to 2026-03-26**: Intermittent double usage windows during March 2026

For windows that partially overlap with a promo period, the adjustment is proportional to the overlap fraction.

## Examples

```bash
# Export to HTML with decimal cost
ccost --per project --cost decimal --output html --filename report.html

# Filter to a specific session
ccost --session abc123 --from 2026-03-23

# UTC timezone, descending order
ccost --tz UTC --order desc --from 2026-03-01

# Filter to opus models only, hide cost column
ccost --model opus --cost false

# Show a 5-hour window starting at a given time
ccost --5hfrom 2026-03-24T10:00:00

# Show a 1-week window ending now
ccost --1wto 2026-03-24T15:00:00

# Use latest LiteLLM pricing (fetched at runtime)
ccost --live-pricing

# Use your own pricing data file
ccost --pricing-data ./my-pricing.json

# Cost chart for last week
ccost --chart cost --1wto 2026-03-25

# Token chart filtered by model
ccost --chart token --model opus --from 2026-03-01

# Export CSV grouped by month
ccost --per month --output csv --filename usage.csv

# Two-level grouping: project then model, full table
ccost --per project --per model --table full

# Group by hour for a single day
ccost --per hour --from 2026-03-23 --to 2026-03-23 --tz UTC

# Copy JSON to clipboard while viewing the table
ccost --copy json

# Copy markdown to clipboard while also writing HTML to file
ccost --output html --copy markdown

# Statusline: session summaries (default view)
ccost sl --cost decimal

# Statusline: rate limit timeline per action
ccost sl --per action

# Statusline: 5h budget estimation
ccost sl --per 5h --cost decimal

# Statusline: compare costs between statusline and LiteLLM
ccost sl --cost-diff --cost decimal

# Statusline: rate limit chart
ccost sl --chart 5h
```

## Library API

ccost is also a Rust library. Add it to your `Cargo.toml`:

```toml
[dependencies]
ccost = { path = "../ccost-rs" }
```

### Pipeline

The processing pipeline follows 4 steps:

```
load_records → calculate_cost → group_records → format_*
```

```rust
use ccost::*;

// 1. Load and parse JSONL records
let options = LoadOptions {
    from: Some("2026-03-01".to_string()),
    to: Some("2026-03-31".to_string()),
    tz: Some("UTC".to_string()),
    model: Some("opus".to_string()),  // optional substring filter
    ..Default::default()
};

let result = load_records(&options);
// result.records: Vec<TokenRecord>
// result.dedup: DedupStats { before, after }
// result.meta: RecordsMeta { earliest, latest, projects, models, sessions }

// 2. Calculate costs using bundled pricing
let pricing = load_pricing();
let priced = calculate_cost(&result.records, Some(&pricing));

// Or use live pricing from LiteLLM (requires network):
// let live_pricing = fetch_live_pricing().unwrap();
// let priced = calculate_cost(&result.records, Some(&live_pricing));

// Or load from a custom file:
// let custom = load_pricing_from_file("./pricing.json").unwrap();

// 3. Group records
let dimensions = vec![GroupDimension::Day, GroupDimension::Model];
let group_options = GroupOptions {
    order: SortOrder::Asc,
    tz: Some("UTC".to_string()),
};
let grouped = group_records(&priced, &dimensions, Some(&group_options));
// grouped.data: Vec<GroupedData> — each with optional children for two-level grouping
// grouped.totals: GroupedData — grand total row

// 4. Format output
use ccost::formatters::table::TableOptions;

let table_opts = TableOptions {
    dimension_label: "Date/Model".to_string(),
    price_mode: PriceMode::Decimal,
    compact: false,
    color: Some(true),
};
let table = format_table(&grouped.data, &grouped.totals, &table_opts);
print!("{}", table);
```

### Key Types

```rust
struct LoadOptions {
    claude_dir: Option<String>,   // override ~/.claude
    from: Option<String>,         // YYYY-MM-DD or YYYY-MM-DDTHH:MM:SS
    to: Option<String>,
    tz: Option<String>,           // UTC, +HH:MM, or IANA name
    project: Option<String>,      // substring filter
    model: Option<String>,
    session: Option<String>,
}

enum GroupDimension { Day, Hour, Month, Session, Project, Model }

struct GroupedData {
    label: String,
    input_tokens: u64,
    output_tokens: u64,
    cache_creation_tokens: u64,
    cache_read_tokens: u64,
    input_cost: f64,
    cache_creation_cost: f64,
    cache_read_cost: f64,
    output_cost: f64,
    total_cost: f64,
    children: Option<Vec<GroupedData>>,  // present in two-level grouping
}

struct RecordsMeta {
    earliest: Option<DateTime<Utc>>,
    latest: Option<DateTime<Utc>>,
    projects: Vec<String>,    // unique sorted project paths
    models: Vec<String>,      // unique sorted model names
    sessions: Vec<String>,    // unique sorted session IDs
}

struct PricingData {
    fetched_at: String,
    models: HashMap<String, ModelPricing>,
}
```

### Available Formatters

| Function | Output |
|----------|--------|
| `format_table()` | Unicode box-drawing table with optional ANSI colors |
| `format_txt()` | Plain-text table (no ANSI colors), convenience wrapper for `format_table()` |
| `format_json()` | JSON with `meta`, `data`, `totals`, `dedup` sections |
| `format_markdown()` | GitHub-flavored Markdown table |
| `format_html()` | Self-contained HTML page with sortable table |
| `format_csv()` | RFC 4180 CSV |
| `format_tsv()` | Tab-separated values |
| `render_chart()` | Braille line chart for terminal |

## How It Works

### Data Source

ccost reads JSONL conversation logs from Claude Code's local storage:
- `~/.claude/projects/*/` — session files and `subagents/` subdirectories
- `~/.config/claude/projects/*/` — alternate location

Both locations are scanned and deduplicated via symlink detection.

### Deduplication

Streaming API entries produce multiple JSONL records with the same `messageId:requestId` composite key but monotonically increasing `output_tokens`. ccost keeps only the record with the **highest** `output_tokens` for each key. This dedup is **global** across all files, which also handles duplicate entries between parent session files and `subagents/` files.

Records without both `messageId` and `requestId` pass through unmodified.

### Pricing

Costs are calculated per-record using a model-to-price lookup with three-tier fuzzy matching:
1. Exact match against pricing keys
2. Strip trailing date suffix (`-YYYYMMDD` or `@YYYYMMDD`), retry exact match
3. Substring containment among `claude-*` pricing keys

Pricing data is bundled at compile time from LiteLLM. Use `--live-pricing` to fetch the latest at runtime or `--pricing-data` to supply your own file.

### Project Path Decoding

Directory names like `-home-user-workspace-test` are decoded to full paths (`/home/user/workspace/test`) by replacing leading hyphens with path separators.

## Development

```bash
cargo test             # run all tests
cargo build --release  # optimized build
cargo run -- --help    # show CLI help
```

## License

MIT
