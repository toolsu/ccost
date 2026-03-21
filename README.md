# cctokens <img src="logo.svg" alt="cctokens" width="35" />

Analyze Claude Code token usage and cost from your local conversation history.

Rust port of [cctokens](https://github.com/tomchen/cctokens) (TypeScript).

Reads JSONL files from `~/.claude/projects/` and `~/.config/claude/projects/`, deduplicates streaming entries, calculates costs using LiteLLM pricing, groups by configurable dimensions, and outputs as table, JSON, Markdown, HTML, CSV, TSV, or braille chart.

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
cctokens

# Show usage for a specific date range
cctokens --from 2026-03-01 --to 2026-03-31

# Group by model only
cctokens --per model

# Group by model with 2-decimal cost breakdown
cctokens --per model --cost decimal

# Group by project for March 2026
cctokens --per project --from 2026-03-01 --to 2026-03-31

# Show cost chart over time
cctokens --chart cost

# Show token usage chart
cctokens --chart token --from 2026-03-01
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
| `--output <fmt>` | Output format: `json`, `markdown`, `html`, `txt`, `csv`, `tsv`. Writes to file (default name: `cctokens.<ext>`). |
| `--filename <path>` | Custom output filename. Works with or without `--output`. |
| `--table <mode>` | Terminal table display: `auto` (default), `full`, `compact`. `auto` switches to compact when terminal width < 120. `compact` hides the `In`, `Cache Cr`, `Cache Rd` columns, showing only `In Total`, `Out`, `Total`. |
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

## Examples

```bash
# Export to HTML with decimal cost
cctokens --per project --cost decimal --output html --filename report.html

# Filter to a specific session
cctokens --session abc123 --from 2026-03-23

# UTC timezone, descending order
cctokens --tz UTC --order desc --from 2026-03-01

# Filter to opus models only, hide cost column
cctokens --model opus --cost false

# Show a 5-hour window starting at a given time
cctokens --5hfrom 2026-03-24T10:00:00

# Show a 1-week window ending now
cctokens --1wto 2026-03-24T15:00:00

# Use latest LiteLLM pricing (fetched at runtime)
cctokens --live-pricing

# Use your own pricing data file
cctokens --pricing-data ./my-pricing.json

# Cost chart for last week
cctokens --chart cost --1wto 2026-03-25

# Token chart filtered by model
cctokens --chart token --model opus --from 2026-03-01

# Export CSV grouped by month
cctokens --per month --output csv --filename usage.csv

# Two-level grouping: project then model, full table
cctokens --per project --per model --table full

# Group by hour for a single day
cctokens --per hour --from 2026-03-23 --to 2026-03-23 --tz UTC

# Copy JSON to clipboard while viewing the table
cctokens --copy json

# Copy markdown to clipboard while also writing HTML to file
cctokens --output html --copy markdown
```

## Library API

cctokens is also a Rust library. Add it to your `Cargo.toml`:

```toml
[dependencies]
cctokens = { path = "../cctokens-rs" }
```

### Pipeline

The processing pipeline follows 4 steps:

```
load_records → calculate_cost → group_records → format_*
```

```rust
use cctokens::*;

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
use cctokens::formatters::table::TableOptions;

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

cctokens reads JSONL conversation logs from Claude Code's local storage:
- `~/.claude/projects/*/` — session files and `subagents/` subdirectories
- `~/.config/claude/projects/*/` — alternate location

Both locations are scanned and deduplicated via symlink detection.

### Deduplication

Streaming API entries produce multiple JSONL records with the same `messageId:requestId` composite key but monotonically increasing `output_tokens`. cctokens keeps only the record with the **highest** `output_tokens` for each key. This dedup is **global** across all files, which also handles duplicate entries between parent session files and `subagents/` files.

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
cargo test             # run all 169 tests
cargo build --release  # optimized build
cargo run -- --help    # show CLI help
```

## License

MIT
