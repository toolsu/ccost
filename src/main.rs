use std::fs;
use std::io::{IsTerminal, Write};
use std::process;

use chrono::NaiveDateTime;
use clap::{Arg, ArgAction, Command};
use regex::Regex;

use cctokens::formatters::chart::{ChartModeEnum, ChartOptions};
use cctokens::formatters::csv::DsvOptions;
use cctokens::formatters::html::HtmlOptions;
use cctokens::formatters::json::JsonMeta;
use cctokens::formatters::markdown::MarkdownOptions;
use cctokens::formatters::table::TableOptions;
use cctokens::*;

const VERSION: &str = "1.0.0";

fn term_width() -> usize {
    // COLUMNS env var takes priority (explicit user/test override)
    if let Ok(cols) = std::env::var("COLUMNS") {
        if let Ok(n) = cols.parse::<usize>() {
            if n > 0 {
                return n;
            }
        }
    }
    // When piped (not a terminal), default to wide
    if !std::io::stdout().is_terminal() {
        return 200;
    }
    // Query actual terminal size
    if let Some((terminal_size::Width(w), _)) = terminal_size::terminal_size() {
        if w > 0 {
            return w as usize;
        }
    }
    80
}

fn print_help() {
    let help = r#"Usage: cctokens [options]

Options:
  --per <dim>           Group by dimension: day, hour, month, session, project, model
                        Can be specified multiple times. Max 2 values.
                        Default: --per day --per model
  --from <date>         Start date (YYYY-MM-DD or YYYY-MM-DDTHH:MM:SS)
  --to <date>           End date (YYYY-MM-DD or YYYY-MM-DDTHH:MM:SS)
  --tz <timezone>       Timezone: UTC, +HH:MM, or IANA name
  --project <name>      Filter by project (substring, case-insensitive)
  --model <name>        Filter by model (substring, case-insensitive)
  --session <id>        Filter by session (substring, case-insensitive)
  --cost <mode>         Cost display: true (default, integer), false (off), decimal (2 d.p.)
  --5hfrom <datetime>   5-hour window from datetime
  --5hto <datetime>     5-hour window to datetime
  --1wfrom <datetime>   1-week window from datetime
  --1wto <datetime>     1-week window to datetime
  --output <format>     Output format: json, markdown, html, txt, csv, tsv
  --filename <path>     Write output to file
  --order <order>       Sort order: asc (default), desc
  --claude-dir <dir>    Override Claude config directory
  --table <mode>        Table mode: auto (default), full, compact
  --live-pricing        Fetch latest pricing from LiteLLM
  --pricing-data <path> Custom pricing file
  --chart <mode>        Chart mode: cost, token
  --copy <format>       Copy to clipboard: json, markdown, html, txt, csv, tsv
  --help                Show this help
  --version             Show version"#;
    eprintln!("{}", help);
}

fn build_command() -> Command {
    Command::new("cctokens")
        .version(VERSION)
        .disable_help_flag(true)
        .disable_version_flag(true)
        .arg(
            Arg::new("per")
                .long("per")
                .action(ArgAction::Append)
                .value_name("dim"),
        )
        .arg(Arg::new("from").long("from").value_name("date"))
        .arg(Arg::new("to").long("to").value_name("date"))
        .arg(Arg::new("tz").long("tz").value_name("timezone"))
        .arg(Arg::new("project").long("project").value_name("name"))
        .arg(Arg::new("model").long("model").value_name("name"))
        .arg(Arg::new("session").long("session").value_name("id"))
        .arg(Arg::new("cost").long("cost").value_name("mode"))
        .arg(Arg::new("5hfrom").long("5hfrom").value_name("datetime"))
        .arg(Arg::new("5hto").long("5hto").value_name("datetime"))
        .arg(Arg::new("1wfrom").long("1wfrom").value_name("datetime"))
        .arg(Arg::new("1wto").long("1wto").value_name("datetime"))
        .arg(Arg::new("output").long("output").value_name("format"))
        .arg(Arg::new("filename").long("filename").value_name("path"))
        .arg(Arg::new("order").long("order").value_name("order"))
        .arg(
            Arg::new("claude-dir")
                .long("claude-dir")
                .value_name("dir"),
        )
        .arg(Arg::new("table").long("table").value_name("mode"))
        .arg(
            Arg::new("live-pricing")
                .long("live-pricing")
                .action(ArgAction::SetTrue),
        )
        .arg(
            Arg::new("pricing-data")
                .long("pricing-data")
                .value_name("path"),
        )
        .arg(Arg::new("chart").long("chart").value_name("mode"))
        .arg(Arg::new("copy").long("copy").value_name("format"))
        .arg(
            Arg::new("help")
                .long("help")
                .action(ArgAction::SetTrue),
        )
        .arg(
            Arg::new("version")
                .long("version")
                .action(ArgAction::SetTrue),
        )
}

/// Parse a datetime string: replace space with T, if 10 chars add T00:00:00.
/// Returns a NaiveDateTime on success.
fn parse_datetime(s: &str) -> Option<NaiveDateTime> {
    let normalized = s.replace(' ', "T");
    let full = if normalized.len() == 10 {
        format!("{}T00:00:00", normalized)
    } else if normalized.len() == 16 {
        // YYYY-MM-DDTHH:MM -> add :00
        format!("{}:00", normalized)
    } else {
        normalized
    };
    NaiveDateTime::parse_from_str(&full, "%Y-%m-%dT%H:%M:%S").ok()
}

fn main() {
    let cmd = build_command();
    let matches = cmd.try_get_matches().unwrap_or_else(|e| {
        eprintln!("{}", e);
        process::exit(1);
    });

    // Handle --help
    if matches.get_flag("help") {
        print_help();
        process::exit(0);
    }

    // Handle --version
    if matches.get_flag("version") {
        println!("cctokens v{}", VERSION);
        process::exit(0);
    }

    // Collect all validation errors
    let mut errors: Vec<String> = Vec::new();

    let date_re = Regex::new(r"^\d{4}-\d{2}-\d{2}([ T]\d{2}:\d{2}(:\d{2})?)?$").unwrap();

    // --per: validate and collect dimensions
    let per_values: Vec<String> = matches
        .get_many::<String>("per")
        .map(|vals| vals.cloned().collect())
        .unwrap_or_default();

    let mut dimensions: Vec<GroupDimension> = Vec::new();

    if per_values.is_empty() {
        dimensions.push(GroupDimension::Day);
        dimensions.push(GroupDimension::Model);
    } else {
        if per_values.len() > 2 {
            errors.push("--per: maximum 2 values allowed".to_string());
        }
        for val in &per_values {
            match GroupDimension::from_str(val) {
                Some(dim) => dimensions.push(dim),
                None => errors.push(format!(
                    "--per: invalid dimension '{}'. Valid: {}",
                    val,
                    GroupDimension::all_valid().join(", ")
                )),
            }
        }
    }

    // --output: validate
    let output_format = matches.get_one::<String>("output").cloned();
    if let Some(ref fmt) = output_format {
        match fmt.as_str() {
            "json" | "markdown" | "html" | "txt" | "csv" | "tsv" => {}
            _ => errors.push(format!(
                "--output: invalid format '{}'. Valid: json, markdown, html, txt, csv, tsv",
                fmt
            )),
        }
    }

    // --order: validate
    let order_str = matches.get_one::<String>("order").cloned();
    let order = if let Some(ref o) = order_str {
        match SortOrder::from_str(o) {
            Some(ord) => ord,
            None => {
                errors.push(format!(
                    "--order: invalid value '{}'. Valid: asc, desc",
                    o
                ));
                SortOrder::Asc
            }
        }
    } else {
        SortOrder::Asc
    };

    // --table: validate
    let table_mode_str = matches.get_one::<String>("table").cloned();
    if let Some(ref mode) = table_mode_str {
        match mode.as_str() {
            "auto" | "full" | "compact" => {}
            _ => errors.push(format!(
                "--table: invalid mode '{}'. Valid: auto, full, compact",
                mode
            )),
        }
    }

    // --copy: validate
    let copy_format = matches.get_one::<String>("copy").cloned();
    if let Some(ref fmt) = copy_format {
        match fmt.as_str() {
            "json" | "markdown" | "html" | "txt" | "csv" | "tsv" => {}
            _ => errors.push(format!(
                "--copy: invalid format '{}'. Valid: json, markdown, html, txt, csv, tsv",
                fmt
            )),
        }
    }

    // --cost: validate
    let cost_str = matches.get_one::<String>("cost").cloned();
    let price_mode = if let Some(ref c) = cost_str {
        match c.as_str() {
            "true" | "" => PriceMode::Integer,
            "false" => PriceMode::Off,
            "decimal" => PriceMode::Decimal,
            _ => {
                errors.push(format!(
                    "--cost: invalid value '{}'. Valid: true, false, decimal",
                    c
                ));
                PriceMode::Integer
            }
        }
    } else {
        PriceMode::Integer
    };

    // --chart: validate
    let chart_str = matches.get_one::<String>("chart").cloned();
    let chart_mode = if let Some(ref c) = chart_str {
        match c.as_str() {
            "cost" => Some(ChartModeEnum::Cost),
            "token" => Some(ChartModeEnum::Token),
            _ => {
                errors.push(format!(
                    "--chart: invalid mode '{}'. Valid: cost, token",
                    c
                ));
                None
            }
        }
    } else {
        None
    };

    // --live-pricing and --pricing-data conflict
    let live_pricing = matches.get_flag("live-pricing");
    let pricing_data_path = matches.get_one::<String>("pricing-data").cloned();
    if live_pricing && pricing_data_path.is_some() {
        errors.push("--live-pricing and --pricing-data cannot be used together".to_string());
    }

    // --chart conflicts with --output (except txt)
    if chart_mode.is_some() {
        if let Some(ref fmt) = output_format {
            if fmt != "txt" {
                errors.push(format!(
                    "--chart conflicts with --output {} (only txt is allowed with --chart)",
                    fmt
                ));
            }
        }
    }

    // Validate date formats and conflicts for --from, --to, --5hfrom, --5hto, --1wfrom, --1wto
    let from_val = matches.get_one::<String>("from").cloned();
    let to_val = matches.get_one::<String>("to").cloned();
    let h5from_val = matches.get_one::<String>("5hfrom").cloned();
    let h5to_val = matches.get_one::<String>("5hto").cloned();
    let w1from_val = matches.get_one::<String>("1wfrom").cloned();
    let w1to_val = matches.get_one::<String>("1wto").cloned();

    // Validate date formats
    for (name, val) in [
        ("--from", &from_val),
        ("--to", &to_val),
        ("--5hfrom", &h5from_val),
        ("--5hto", &h5to_val),
        ("--1wfrom", &w1from_val),
        ("--1wto", &w1to_val),
    ] {
        if let Some(ref v) = val {
            if !date_re.is_match(v) {
                errors.push(format!(
                    "{}: invalid date format '{}'. Expected YYYY-MM-DD or YYYY-MM-DDTHH:MM:SS",
                    name, v
                ));
            }
        }
    }

    // --5hfrom and --5hto conflict with each other
    if h5from_val.is_some() && h5to_val.is_some() {
        errors.push("--5hfrom and --5hto cannot be used together".to_string());
    }

    // --1wfrom and --1wto conflict with each other
    if w1from_val.is_some() && w1to_val.is_some() {
        errors.push("--1wfrom and --1wto cannot be used together".to_string());
    }

    // --5h* and --1w* conflict with each other
    let has_5h = h5from_val.is_some() || h5to_val.is_some();
    let has_1w = w1from_val.is_some() || w1to_val.is_some();
    if has_5h && has_1w {
        errors.push("--5h* and --1w* options cannot be used together".to_string());
    }

    // --5h*/--1w* conflict with --from/--to
    if (has_5h || has_1w) && (from_val.is_some() || to_val.is_some()) {
        errors.push(
            "--5hfrom/--5hto/--1wfrom/--1wto cannot be used with --from/--to".to_string(),
        );
    }

    // Exit if any errors collected
    if !errors.is_empty() {
        for err in &errors {
            eprintln!("Error: {}", err);
        }
        eprintln!();
        print_help();
        process::exit(1);
    }

    // Compute effective from/to from time window shortcuts
    let (effective_from, effective_to) = compute_date_range(
        from_val, to_val, h5from_val, h5to_val, w1from_val, w1to_val,
    );

    // Load pricing
    let pricing = if live_pricing {
        eprintln!("Fetching live pricing from LiteLLM...");
        match fetch_live_pricing() {
            Ok(p) => {
                eprintln!("Fetched pricing for {} models", p.models.len());
                p
            }
            Err(e) => {
                eprintln!("Error fetching live pricing: {}", e);
                process::exit(1);
            }
        }
    } else if let Some(ref path) = pricing_data_path {
        match load_pricing_from_file(path) {
            Ok(p) => p,
            Err(e) => {
                eprintln!("Error loading pricing from '{}': {}", path, e);
                process::exit(1);
            }
        }
    } else {
        load_pricing()
    };

    // Determine pricing date for footer
    let pricing_date = {
        // Extract date portion from fetched_at (first 10 chars)
        if pricing.fetched_at.len() >= 10 {
            pricing.fetched_at[..10].to_string()
        } else {
            pricing.fetched_at.clone()
        }
    };

    // Load records
    let load_opts = LoadOptions {
        claude_dir: matches.get_one::<String>("claude-dir").cloned(),
        from: effective_from,
        to: effective_to,
        tz: matches.get_one::<String>("tz").cloned(),
        project: matches.get_one::<String>("project").cloned(),
        model: matches.get_one::<String>("model").cloned(),
        session: matches.get_one::<String>("session").cloned(),
    };

    let load_result = load_records(&load_opts);
    let dedup = load_result.dedup;
    let meta = load_result.meta;

    // Calculate costs
    let priced_records = calculate_cost(&load_result.records, Some(&pricing));

    let tz_opt = matches.get_one::<String>("tz").cloned();
    let group_opts = GroupOptions {
        order,
        tz: tz_opt.clone(),
    };

    let filename_opt = matches.get_one::<String>("filename").cloned();

    // Build footer lines
    let dedup_removed = dedup.before - dedup.after;
    let dedup_line = format!(
        "Streaming dedup: {} duplicates removed ({} \u{2192} {})",
        dedup_removed, dedup.before, dedup.after
    );
    let pricing_line = format!("Prices: LiteLLM ({})", pricing_date);
    let footer = format!("{}\n{}", dedup_line, pricing_line);

    // Shared computations for both chart and non-chart paths
    let group_result = group_records(&priced_records, &dimensions, Some(&group_opts));

    let dimension_label = dimensions
        .iter()
        .map(|d| d.label())
        .collect::<Vec<_>>()
        .join("/");

    let table_mode = table_mode_str.as_deref().unwrap_or("auto");
    let compact = match table_mode {
        "compact" => true,
        "full" => false,
        _ => {
            // auto: compact if terminal width < 120
            term_width() < 120
        }
    };

    // Helper: generate formatted content for a given format
    let generate_format_content = |fmt: &str| -> String {
        match fmt {
            "json" => {
                let json_meta = JsonMeta {
                    dimensions: dimensions.iter().map(|d| d.as_str().to_string()).collect(),
                    from: load_opts.from.clone(),
                    to: load_opts.to.clone(),
                    tz: tz_opt.clone(),
                    project: load_opts.project.clone(),
                    model: load_opts.model.clone(),
                    session: load_opts.session.clone(),
                    order: if order == SortOrder::Desc {
                        "desc".to_string()
                    } else {
                        "asc".to_string()
                    },
                    earliest: meta.earliest.map(|d| d.to_rfc3339()),
                    latest: meta.latest.map(|d| d.to_rfc3339()),
                    projects: meta.projects.clone(),
                    models: meta.models.clone(),
                    sessions: meta.sessions.clone(),
                    generated_at: chrono::Utc::now().to_rfc3339(),
                    pricing_date: pricing_date.clone(),
                };
                format_json(
                    &group_result.data,
                    &group_result.totals,
                    &json_meta,
                    &dedup,
                )
            }
            "markdown" => {
                let md_opts = MarkdownOptions {
                    dimension_label: dimension_label.clone(),
                    price_mode,
                    compact,
                };
                format_markdown(&group_result.data, &group_result.totals, &md_opts)
            }
            "html" => {
                let html_opts = HtmlOptions {
                    dimension_label: dimension_label.clone(),
                    price_mode,
                    compact,
                    title: None,
                };
                format_html(&group_result.data, &group_result.totals, &html_opts)
            }
            "txt" => {
                let table_opts = TableOptions {
                    dimension_label: dimension_label.clone(),
                    price_mode,
                    compact,
                    color: Some(false),
                };
                let table_str =
                    format_table(&group_result.data, &group_result.totals, &table_opts);
                format!("{}{}\n", table_str, footer)
            }
            "csv" => {
                let dsv_opts = DsvOptions {
                    dimension_label: dimension_label.clone(),
                    price_mode,
                    compact,
                };
                format_csv(&group_result.data, &group_result.totals, &dsv_opts)
            }
            "tsv" => {
                let dsv_opts = DsvOptions {
                    dimension_label: dimension_label.clone(),
                    price_mode,
                    compact,
                };
                format_tsv(&group_result.data, &group_result.totals, &dsv_opts)
            }
            _ => unreachable!(),
        }
    };

    // Handle --copy (works alongside all other flags)
    if let Some(ref copy_fmt) = copy_format {
        let copy_content = generate_format_content(copy_fmt);
        match copy_to_clipboard(&copy_content) {
            Ok(()) => eprintln!("Copied {} to clipboard", copy_fmt),
            Err(e) => eprintln!("Error: {}", e),
        }
    }

    // Chart mode
    if let Some(chart_mode_val) = chart_mode {
        let chart_opts = ChartOptions {
            mode: chart_mode_val,
            dimension_label: dimensions
                .first()
                .map(|d| d.label().to_string())
                .unwrap_or_else(|| "Date".to_string()),
            price_mode,
            tz: tz_opt.clone(),
            width: None,
            height: None,
        };

        let chart_output = render_chart(&group_result.data, &group_result.totals, &chart_opts);

        if output_format.is_some() || filename_opt.is_some() {
            // Write to file
            let ext = output_format
                .as_deref()
                .unwrap_or("txt");
            let target_filename = filename_opt.unwrap_or_else(|| format!("cctokens.{}", ext));
            let file_content = format!("{}\n{}\n", chart_output, footer);
            if let Err(e) = fs::write(&target_filename, &file_content) {
                eprintln!("Error writing to '{}': {}", target_filename, e);
                process::exit(1);
            }
            eprintln!("Wrote report to {}", target_filename);
        } else {
            // Print chart to stdout
            print!("{}", chart_output);
        }

        // Footer always to stderr
        eprintln!("{}", footer);
        return;
    }

    // Non-chart mode

    // No --output and no --filename: print table to stdout with ANSI colors
    if output_format.is_none() && filename_opt.is_none() {
        let table_opts = TableOptions {
            dimension_label: dimension_label.clone(),
            price_mode,
            compact,
            color: Some(true),
        };

        let table_str = format_table(&group_result.data, &group_result.totals, &table_opts);
        print!("{}", table_str);
        eprintln!("{}", footer);
        return;
    }

    // Extension mapping
    fn ext_for_format(fmt: &str) -> &str {
        match fmt {
            "markdown" => "md",
            "json" => "json",
            "html" => "html",
            "txt" => "txt",
            "csv" => "csv",
            "tsv" => "tsv",
            _ => fmt,
        }
    }

    // Determine output content
    let output_fmt = output_format.as_deref();
    let file_content = match output_fmt {
        Some(fmt) => generate_format_content(fmt),
        None => {
            // --filename without --output: write plain text table (no ANSI) + footer
            let table_opts = TableOptions {
                dimension_label: dimension_label.clone(),
                price_mode,
                compact,
                color: Some(false),
            };
            let table_str =
                format_table(&group_result.data, &group_result.totals, &table_opts);
            format!("{}{}\n", table_str, footer)
        }
    };

    // Determine filename
    let ext = output_fmt
        .map(|f| ext_for_format(f))
        .unwrap_or("txt");
    let target_filename = filename_opt.unwrap_or_else(|| format!("cctokens.{}", ext));

    if let Err(e) = fs::write(&target_filename, &file_content) {
        eprintln!("Error writing to '{}': {}", target_filename, e);
        process::exit(1);
    }

    eprintln!("Wrote report to {}", target_filename);
    eprintln!("{}", footer);
}

/// Compute effective (from, to) date strings from the various date options.
fn compute_date_range(
    from_val: Option<String>,
    to_val: Option<String>,
    h5from_val: Option<String>,
    h5to_val: Option<String>,
    w1from_val: Option<String>,
    w1to_val: Option<String>,
) -> (Option<String>, Option<String>) {
    if let Some(ref val) = h5from_val {
        if let Some(dt) = parse_datetime(val) {
            let end = dt + chrono::Duration::hours(5);
            return (
                Some(dt.format("%Y-%m-%dT%H:%M:%S").to_string()),
                Some(end.format("%Y-%m-%dT%H:%M:%S").to_string()),
            );
        }
        return (from_val, to_val);
    }

    if let Some(ref val) = h5to_val {
        if let Some(dt) = parse_datetime(val) {
            let start = dt - chrono::Duration::hours(5);
            return (
                Some(start.format("%Y-%m-%dT%H:%M:%S").to_string()),
                Some(dt.format("%Y-%m-%dT%H:%M:%S").to_string()),
            );
        }
        return (from_val, to_val);
    }

    if let Some(ref val) = w1from_val {
        if let Some(dt) = parse_datetime(val) {
            let end = dt + chrono::Duration::days(7);
            return (
                Some(dt.format("%Y-%m-%dT%H:%M:%S").to_string()),
                Some(end.format("%Y-%m-%dT%H:%M:%S").to_string()),
            );
        }
        return (from_val, to_val);
    }

    if let Some(ref val) = w1to_val {
        if let Some(dt) = parse_datetime(val) {
            let start = dt - chrono::Duration::days(7);
            return (
                Some(start.format("%Y-%m-%dT%H:%M:%S").to_string()),
                Some(dt.format("%Y-%m-%dT%H:%M:%S").to_string()),
            );
        }
        return (from_val, to_val);
    }

    (from_val, to_val)
}

/// Base64-encode bytes (standard alphabet, with padding).
fn base64_encode(input: &[u8]) -> String {
    const CHARS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut result = String::with_capacity((input.len() + 2) / 3 * 4);
    for chunk in input.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = if chunk.len() > 1 { chunk[1] as u32 } else { 0 };
        let b2 = if chunk.len() > 2 { chunk[2] as u32 } else { 0 };
        let triple = (b0 << 16) | (b1 << 8) | b2;
        result.push(CHARS[((triple >> 18) & 0x3F) as usize] as char);
        result.push(CHARS[((triple >> 12) & 0x3F) as usize] as char);
        if chunk.len() > 1 {
            result.push(CHARS[((triple >> 6) & 0x3F) as usize] as char);
        } else {
            result.push('=');
        }
        if chunk.len() > 2 {
            result.push(CHARS[(triple & 0x3F) as usize] as char);
        } else {
            result.push('=');
        }
    }
    result
}

/// Copy text to clipboard via OSC 52 terminal escape sequence.
/// Works in most modern terminals without any external tools.
fn osc52_copy(text: &str) -> Result<(), String> {
    let encoded = base64_encode(text.as_bytes());
    // Write OSC 52 to stderr (which is typically the terminal)
    eprint!("\x1b]52;c;{}\x07", encoded);
    Ok(())
}

/// Copy text to the system clipboard.
/// Tries native clipboard tools first, falls back to OSC 52.
fn copy_to_clipboard(text: &str) -> Result<(), String> {
    let result = if cfg!(target_os = "macos") {
        process::Command::new("pbcopy")
            .stdin(process::Stdio::piped())
            .stdout(process::Stdio::null())
            .stderr(process::Stdio::null())
            .spawn()
    } else if cfg!(target_os = "windows") {
        process::Command::new("clip")
            .stdin(process::Stdio::piped())
            .stdout(process::Stdio::null())
            .stderr(process::Stdio::null())
            .spawn()
    } else {
        // Linux: try xclip, xsel, wl-copy in order
        process::Command::new("xclip")
            .args(["-selection", "clipboard"])
            .stdin(process::Stdio::piped())
            .stdout(process::Stdio::null())
            .stderr(process::Stdio::null())
            .spawn()
            .or_else(|_| {
                process::Command::new("xsel")
                    .arg("--clipboard")
                    .stdin(process::Stdio::piped())
                    .stdout(process::Stdio::null())
                    .stderr(process::Stdio::null())
                    .spawn()
            })
            .or_else(|_| {
                process::Command::new("wl-copy")
                    .stdin(process::Stdio::piped())
                    .stdout(process::Stdio::null())
                    .stderr(process::Stdio::null())
                    .spawn()
            })
    };

    match result {
        Ok(mut child) => {
            if let Some(mut stdin) = child.stdin.take() {
                stdin
                    .write_all(text.as_bytes())
                    .map_err(|e| format!("Failed to write to clipboard: {}", e))?;
                drop(stdin); // Close stdin so the child sees EOF
            }
            let status = child
                .wait()
                .map_err(|e| format!("Clipboard process failed: {}", e))?;
            if status.success() {
                Ok(())
            } else {
                // Native tool failed, fall back to OSC 52
                osc52_copy(text)
            }
        }
        Err(_) => {
            // No native tool found, fall back to OSC 52
            osc52_copy(text)
        }
    }
}
