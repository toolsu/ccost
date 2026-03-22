use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::process;

use clap::{Arg, ArgAction, Command};
use regex::Regex;

use ccost::formatters::chart::{render_chart_raw, y_label_cost, y_label_percent, ChartModeEnum, ChartOptions};
use ccost::formatters::csv::DsvOptions;
use ccost::formatters::html::HtmlOptions;
use ccost::formatters::json::JsonMeta;
use ccost::formatters::markdown::MarkdownOptions;
use ccost::formatters::table::TableOptions;
use ccost::sl::formatter::*;
use ccost::sl::{
    aggregate_by_day, aggregate_by_project, aggregate_ratelimit, aggregate_sessions,
    aggregate_windows, load_sl_records, SlChartMode, SlCostDiff, SlLoadOptions, SlViewMode,
    WindowType,
};
use ccost::utils::{compute_date_range, copy_to_clipboard, ext_for_format, term_width};
use ccost::*;

const VERSION: &str = "1.0.0";

fn print_help() {
    let help = r#"Usage: ccost [options]

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
  --version             Show version

Subcommands:
  sl                    Analyze Claude statusline data (rate limits, sessions, costs)"#;
    eprintln!("{}", help);
}

fn build_command() -> Command {
    Command::new("ccost")
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
        .subcommand(build_sl_subcommand())
}

fn build_sl_subcommand() -> Command {
    Command::new("sl")
        .about("Analyze Claude statusline data")
        .disable_help_flag(true)
        .arg(Arg::new("file").long("file").value_name("path"))
        .arg(Arg::new("per").long("per").value_name("dim"))
        .arg(Arg::new("chart").long("chart").value_name("mode"))
        .arg(
            Arg::new("cost-diff")
                .long("cost-diff")
                .action(ArgAction::SetTrue),
        )
        .arg(Arg::new("from").long("from").value_name("date"))
        .arg(Arg::new("to").long("to").value_name("date"))
        .arg(Arg::new("tz").long("tz").value_name("timezone"))
        .arg(Arg::new("session").long("session").value_name("id"))
        .arg(Arg::new("project").long("project").value_name("name"))
        .arg(Arg::new("model").long("model").value_name("name"))
        .arg(Arg::new("cost").long("cost").value_name("mode"))
        .arg(Arg::new("output").long("output").value_name("format"))
        .arg(Arg::new("filename").long("filename").value_name("path"))
        .arg(Arg::new("copy").long("copy").value_name("format"))
        .arg(Arg::new("order").long("order").value_name("order"))
        .arg(Arg::new("table").long("table").value_name("mode"))
        .arg(Arg::new("5hfrom").long("5hfrom").value_name("datetime"))
        .arg(Arg::new("5hto").long("5hto").value_name("datetime"))
        .arg(Arg::new("1wfrom").long("1wfrom").value_name("datetime"))
        .arg(Arg::new("1wto").long("1wto").value_name("datetime"))
        .arg(
            Arg::new("nopromo")
                .long("nopromo")
                .action(ArgAction::SetTrue),
        )
        .arg(
            Arg::new("claude-dir")
                .long("claude-dir")
                .value_name("dir"),
        )
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
        .arg(
            Arg::new("help")
                .long("help")
                .action(ArgAction::SetTrue),
        )
}

fn main() {
    let cmd = build_command();
    let matches = cmd.try_get_matches().unwrap_or_else(|e| {
        eprintln!("{}", e);
        process::exit(1);
    });

    // Handle sl subcommand
    if let Some(sl_matches) = matches.subcommand_matches("sl") {
        run_sl(sl_matches);
        return;
    }

    // Handle --help
    if matches.get_flag("help") {
        print_help();
        process::exit(0);
    }

    // Handle --version
    if matches.get_flag("version") {
        println!("ccost v{}", VERSION);
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
    let has_file_output = output_format.is_some() || filename_opt.is_some();
    let compact = match table_mode {
        "compact" => true,
        "full" => false,
        _ => {
            // auto: full when writing to file, compact if terminal width < 120
            if has_file_output { false } else { term_width() < 120 }
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
            let target_filename = filename_opt.unwrap_or_else(|| format!("ccost.{}", ext));
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
    let target_filename = filename_opt.unwrap_or_else(|| format!("ccost.{}", ext));

    if let Err(e) = fs::write(&target_filename, &file_content) {
        eprintln!("Error writing to '{}': {}", target_filename, e);
        process::exit(1);
    }

    eprintln!("Wrote report to {}", target_filename);
    eprintln!("{}", footer);
}

// ─── sl subcommand ────────────────────────────────────────────────────────────

fn print_sl_help() {
    let help = r#"Usage: ccost sl [options]

Analyze Claude statusline data (rate limits, sessions, costs).

Options:
  --file <path>         Path to statusline.jsonl (default: ~/.claude/statusline.jsonl)
  --per <dim>           Group by: action, session, project, day, 1h, 5h (default), 1w
                        Default (no --per): rate-limit timeline
  --nopromo             Disable promo adjustment for Est Budget (default: adjusts for 2x promo periods)
  --chart <mode>        Chart mode: 5h, 1w, cost
  --cost-diff           Compare SL costs with LiteLLM pricing (requires --per session)
  --from <date>         Start date (YYYY-MM-DD or YYYY-MM-DDTHH:MM:SS)
  --to <date>           End date (YYYY-MM-DD or YYYY-MM-DDTHH:MM:SS)
  --tz <timezone>       Timezone: UTC, +HH:MM, or IANA name
  --session <id>        Filter by session (substring, case-insensitive)
  --project <name>      Filter by project (substring, case-insensitive)
  --model <name>        Filter by model (substring, case-insensitive)
  --cost <mode>         Cost display: true (default, integer), false (off), decimal (2 d.p.)
  --output <format>     Output format: json, markdown, html, txt, csv, tsv
  --filename <path>     Write output to file
  --copy <format>       Copy to clipboard: json, markdown, html, txt, csv, tsv
  --order <order>       Sort order: asc (default), desc
  --table <mode>        Table mode: auto (default), full, compact
  --5hfrom <datetime>   5-hour window from datetime
  --5hto <datetime>     5-hour window to datetime
  --1wfrom <datetime>   1-week window from datetime
  --1wto <datetime>     1-week window to datetime
  --claude-dir <dir>    Override Claude config directory
  --live-pricing        Fetch latest pricing from LiteLLM (for --cost-diff)
  --pricing-data <path> Custom pricing file (for --cost-diff)
  --help                Show this help"#;
    eprintln!("{}", help);
}

fn run_sl(matches: &clap::ArgMatches) {
    // Handle --help
    if matches.get_flag("help") {
        print_sl_help();
        return;
    }

    let mut errors: Vec<String> = Vec::new();
    let date_re = Regex::new(r"^\d{4}-\d{2}-\d{2}([ T]\d{2}:\d{2}(:\d{2})?)?$").unwrap();

    // --per: validate (single value for sl)
    let per_val = matches.get_one::<String>("per").cloned();
    let view_mode = if let Some(ref p) = per_val {
        match p.as_str() {
            "action" => SlViewMode::Action,
            "session" => SlViewMode::Session,
            "project" => SlViewMode::Project,
            "day" => SlViewMode::Day,
            "1h" => SlViewMode::Window1h,
            "5h" => SlViewMode::Window5h,
            "1w" => SlViewMode::Window1w,
            _ => {
                errors.push(format!(
                    "--per: invalid dimension '{}'. Valid: action, session, project, day, 1h, 5h, 1w",
                    p
                ));
                SlViewMode::Window5h
            }
        }
    } else {
        SlViewMode::Window5h
    };

    // --chart: validate
    let chart_str = matches.get_one::<String>("chart").cloned();
    let chart_mode = if let Some(ref c) = chart_str {
        match c.as_str() {
            "5h" => Some(SlChartMode::FiveHour),
            "1w" => Some(SlChartMode::OneWeek),
            "cost" => Some(SlChartMode::Cost),
            _ => {
                errors.push(format!(
                    "--chart: invalid mode '{}'. Valid: 5h, 1w, cost",
                    c
                ));
                None
            }
        }
    } else {
        None
    };

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

    // --output: validate
    let output_format = matches.get_one::<String>("output").cloned();
    if let Some(ref fmt) = output_format {
        match fmt.as_str() {
            "json" | "csv" | "tsv" | "txt" | "markdown" | "html" => {}
            _ => errors.push(format!(
                "--output: invalid format '{}'. Valid: json, markdown, html, txt, csv, tsv",
                fmt
            )),
        }
    }

    // --copy: validate
    let copy_format = matches.get_one::<String>("copy").cloned();
    if let Some(ref fmt) = copy_format {
        match fmt.as_str() {
            "json" | "csv" | "tsv" | "txt" | "markdown" | "html" => {}
            _ => errors.push(format!(
                "--copy: invalid format '{}'. Valid: json, markdown, html, txt, csv, tsv",
                fmt
            )),
        }
    }

    // --order: validate
    let order_str = matches.get_one::<String>("order").cloned();
    let _order = if let Some(ref o) = order_str {
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

    // Validate date formats
    let from_val = matches.get_one::<String>("from").cloned();
    let to_val = matches.get_one::<String>("to").cloned();
    let h5from_val = matches.get_one::<String>("5hfrom").cloned();
    let h5to_val = matches.get_one::<String>("5hto").cloned();
    let w1from_val = matches.get_one::<String>("1wfrom").cloned();
    let w1to_val = matches.get_one::<String>("1wto").cloned();

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

    // Date conflicts
    if h5from_val.is_some() && h5to_val.is_some() {
        errors.push("--5hfrom and --5hto cannot be used together".to_string());
    }
    if w1from_val.is_some() && w1to_val.is_some() {
        errors.push("--1wfrom and --1wto cannot be used together".to_string());
    }
    let has_5h = h5from_val.is_some() || h5to_val.is_some();
    let has_1w = w1from_val.is_some() || w1to_val.is_some();
    if has_5h && has_1w {
        errors.push("--5h* and --1w* options cannot be used together".to_string());
    }
    if (has_5h || has_1w) && (from_val.is_some() || to_val.is_some()) {
        errors.push(
            "--5hfrom/--5hto/--1wfrom/--1wto cannot be used with --from/--to".to_string(),
        );
    }

    // --cost-diff warning
    let cost_diff = matches.get_flag("cost-diff");
    if cost_diff && view_mode != SlViewMode::Session {
        eprintln!("Warning: --cost-diff is only effective with --per session");
    }

    // --nopromo (default: promo adjustment enabled)
    let promo = !matches.get_flag("nopromo");

    // Exit if any errors
    if !errors.is_empty() {
        for err in &errors {
            eprintln!("Error: {}", err);
        }
        eprintln!();
        print_sl_help();
        process::exit(1);
    }

    // Compute effective date range
    let (effective_from, effective_to) = compute_date_range(
        from_val, to_val, h5from_val, h5to_val, w1from_val, w1to_val,
    );

    let tz_opt = matches.get_one::<String>("tz").cloned();

    // Determine file path
    let file_path = matches
        .get_one::<String>("file")
        .cloned()
        .unwrap_or_else(|| {
            let home = dirs::home_dir().unwrap_or_default();
            home.join(".claude")
                .join("statusline.jsonl")
                .to_string_lossy()
                .to_string()
        });

    // Check file existence
    if !Path::new(&file_path).exists() {
        eprintln!("Error: statusline file not found: {}", file_path);
        process::exit(1);
    }

    // Load records
    let load_opts = SlLoadOptions {
        file: Some(file_path.clone()),
        from: effective_from.clone(),
        to: effective_to.clone(),
        tz: tz_opt.clone(),
        session: matches.get_one::<String>("session").cloned(),
        project: matches.get_one::<String>("project").cloned(),
        model: matches.get_one::<String>("model").cloned(),
    };

    let (records, skipped) = load_sl_records(&file_path, &load_opts);
    eprintln!("Loaded {} statusline records", records.len());
    if skipped > 0 {
        eprintln!("Skipped {} malformed lines", skipped);
    }

    let filename_opt = matches.get_one::<String>("filename").cloned();

    // Table mode
    let table_mode = table_mode_str.as_deref().unwrap_or("auto");
    let has_file_output = output_format.is_some() || filename_opt.is_some();
    let compact = match table_mode {
        "compact" => true,
        "full" => false,
        _ => {
            if has_file_output { false } else { term_width() < 120 }
        }
    };

    let fmt_opts = SlFormatOptions {
        tz: tz_opt.clone(),
        price_mode,
        compact,
        color: true,
    };

    let fmt_opts_nocolor = SlFormatOptions {
        tz: tz_opt.clone(),
        price_mode,
        compact,
        color: false,
    };

    let json_meta = SlJsonMeta {
        source: "ccost-sl".to_string(),
        file: file_path.clone(),
        view: match view_mode {
            SlViewMode::Action => "action".to_string(),
            SlViewMode::Session => "session".to_string(),
            SlViewMode::Project => "project".to_string(),
            SlViewMode::Day => "day".to_string(),
            SlViewMode::Window1h => "1h".to_string(),
            SlViewMode::Window5h => "5h".to_string(),
            SlViewMode::Window1w => "1w".to_string(),
        },
        from: effective_from.clone(),
        to: effective_to.clone(),
        tz: tz_opt.clone(),
        generated_at: chrono::Utc::now().to_rfc3339(),
    };

    // Chart mode
    if let Some(chart) = chart_mode {
        let (keys, values, title, y_label_fn): (Vec<String>, Vec<f64>, String, fn(f64) -> String) =
            match chart {
                SlChartMode::FiveHour => {
                    let entries = aggregate_ratelimit(&records);
                    let k: Vec<String> = entries
                        .iter()
                        .map(|e| fmt_dt(&e.ts, tz_opt.as_deref(), "%m-%dT%H:%M"))
                        .collect();
                    let v: Vec<f64> = entries.iter().map(|e| e.five_hour_pct as f64).collect();
                    (k, v, "5-Hour Rate Limit %".to_string(), y_label_percent)
                }
                SlChartMode::OneWeek => {
                    let entries = aggregate_ratelimit(&records);
                    let k: Vec<String> = entries
                        .iter()
                        .map(|e| fmt_dt(&e.ts, tz_opt.as_deref(), "%m-%dT%H:%M"))
                        .collect();
                    let v: Vec<f64> = entries.iter().map(|e| e.seven_day_pct as f64).collect();
                    (k, v, "1-Week Rate Limit %".to_string(), y_label_percent)
                }
                SlChartMode::Cost => {
                    let sessions = aggregate_sessions(&records);
                    // Cumulative cost
                    let mut cumulative = 0.0_f64;
                    let k: Vec<String> = sessions
                        .iter()
                        .map(|s| fmt_dt(&s.last_ts, tz_opt.as_deref(), "%m-%dT%H:%M"))
                        .collect();
                    let v: Vec<f64> = sessions
                        .iter()
                        .map(|s| {
                            cumulative += s.total_cost;
                            cumulative
                        })
                        .collect();
                    (k, v, "Cumulative Cost (USD)".to_string(), y_label_cost)
                }
            };

        let chart_output = render_chart_raw(&keys, &values, &title, y_label_fn, None, None);

        // Handle --copy for chart
        if let Some(ref copy_fmt) = copy_format {
            let copy_content = if copy_fmt == "txt" {
                chart_output.clone()
            } else {
                chart_output.clone()
            };
            match copy_to_clipboard(&copy_content) {
                Ok(()) => eprintln!("Copied {} to clipboard", copy_fmt),
                Err(e) => eprintln!("Error: {}", e),
            }
        }

        if output_format.is_some() || filename_opt.is_some() {
            let ext = output_format.as_deref().unwrap_or("txt");
            let target_filename =
                filename_opt.unwrap_or_else(|| format!("ccost-sl.{}", ext));
            let file_content = format!("{}\n", chart_output);
            if let Err(e) = fs::write(&target_filename, &file_content) {
                eprintln!("Error writing to '{}': {}", target_filename, e);
                process::exit(1);
            }
            eprintln!("Wrote report to {}", target_filename);
        } else {
            print!("{}", chart_output);
        }
        return;
    }

    // Non-chart mode: generate table/json/csv content based on view_mode

    // Helper to generate content for a given format string
    let generate_sl_content = |fmt: &str,
                               view: &SlViewMode,
                               recs: &[ccost::sl::SlRecord],
                               color: bool|
     -> String {
        let opts = SlFormatOptions {
            tz: tz_opt.clone(),
            price_mode,
            compact,
            color,
        };

        match fmt {
            "json" => match view {
                SlViewMode::Action => {
                    let entries = aggregate_ratelimit(recs);
                    format_sl_json_ratelimit(&entries, &json_meta)
                }
                SlViewMode::Session => {
                    let sessions = aggregate_sessions(recs);
                    format_sl_json_sessions(&sessions, &json_meta)
                }
                SlViewMode::Project => {
                    let sessions = aggregate_sessions(recs);
                    let projects = aggregate_by_project(&sessions);
                    format_sl_json_projects(&projects, &json_meta)
                }
                SlViewMode::Day => {
                    let sessions = aggregate_sessions(recs);
                    let days = aggregate_by_day(&sessions, tz_opt.as_deref());
                    format_sl_json_days(&days, &json_meta)
                }
                SlViewMode::Window1h => {
                    let sessions = aggregate_sessions(recs);
                    let windows = aggregate_windows(recs, &sessions, WindowType::OneHour, promo);
                    format_sl_json_windows(&windows, &json_meta)
                }
                SlViewMode::Window5h => {
                    let sessions = aggregate_sessions(recs);
                    let windows = aggregate_windows(recs, &sessions, WindowType::FiveHour, promo);
                    format_sl_json_windows(&windows, &json_meta)
                }
                SlViewMode::Window1w => {
                    let sessions = aggregate_sessions(recs);
                    let windows = aggregate_windows(recs, &sessions, WindowType::OneWeek, promo);
                    format_sl_json_windows(&windows, &json_meta)
                }
            },
            "csv" => match view {
                SlViewMode::Action => {
                    let entries = aggregate_ratelimit(recs);
                    format_sl_csv_ratelimit(&entries, tz_opt.as_deref())
                }
                SlViewMode::Session => {
                    let sessions = aggregate_sessions(recs);
                    format_sl_csv_sessions(&sessions, &opts)
                }
                _ => {
                    generate_sl_table_content(view, recs, &opts, cost_diff, promo, matches)
                }
            },
            "markdown" | "html" | "tsv" => {
                let (headers, rows, totals) =
                    generate_sl_table_data(view, recs, &opts, cost_diff, promo, matches);
                let totals_ref = totals.as_ref();
                match fmt {
                    "markdown" => render_markdown(&headers, &rows, totals_ref),
                    "html" => render_html(&headers, &rows, totals_ref),
                    "tsv" => render_tsv(&headers, &rows, totals_ref),
                    _ => unreachable!(),
                }
            },
            "txt" | _ => generate_sl_table_content(view, recs, &opts, cost_diff, promo, matches),
        }
    };

    // Handle --copy
    if let Some(ref copy_fmt) = copy_format {
        let copy_content = generate_sl_content(copy_fmt, &view_mode, &records, false);
        match copy_to_clipboard(&copy_content) {
            Ok(()) => eprintln!("Copied {} to clipboard", copy_fmt),
            Err(e) => eprintln!("Error: {}", e),
        }
    }

    // Output routing
    if output_format.is_none() && filename_opt.is_none() {
        // Print colored table to stdout
        let table_str =
            generate_sl_table_content(&view_mode, &records, &fmt_opts, cost_diff, promo, matches);
        print!("{}", table_str);
        return;
    }

    // Write to file
    let output_fmt = output_format.as_deref();
    let file_content = match output_fmt {
        Some(fmt) => generate_sl_content(fmt, &view_mode, &records, false),
        None => {
            // --filename without --output: write plain text table (no ANSI)
            generate_sl_table_content(&view_mode, &records, &fmt_opts_nocolor, cost_diff, promo, matches)
        }
    };

    let ext = output_fmt.map(|f| ext_for_format(f)).unwrap_or("txt");
    let target_filename = filename_opt.unwrap_or_else(|| format!("ccost-sl.{}", ext));

    if let Err(e) = fs::write(&target_filename, &file_content) {
        eprintln!("Error writing to '{}': {}", target_filename, e);
        process::exit(1);
    }
    eprintln!("Wrote report to {}", target_filename);
}

fn generate_sl_table_content(
    view_mode: &SlViewMode,
    records: &[ccost::sl::SlRecord],
    opts: &SlFormatOptions,
    cost_diff: bool,
    promo: bool,
    matches: &clap::ArgMatches,
) -> String {
    match view_mode {
        SlViewMode::Action => {
            let entries = aggregate_ratelimit(records);
            format_sl_ratelimit_table(&entries, opts)
        }
        SlViewMode::Session => {
            let sessions = aggregate_sessions(records);
            if cost_diff {
                let diffs = compute_cost_diffs(&sessions, matches);
                format_sl_cost_diff_table(&sessions, &diffs, opts)
            } else {
                format_sl_session_table(&sessions, opts)
            }
        }
        SlViewMode::Project => {
            let sessions = aggregate_sessions(records);
            let projects = aggregate_by_project(&sessions);
            format_sl_project_table(&projects, opts)
        }
        SlViewMode::Day => {
            let sessions = aggregate_sessions(records);
            let days = aggregate_by_day(&sessions, opts.tz.as_deref());
            format_sl_day_table(&days, opts)
        }
        SlViewMode::Window1h => {
            let sessions = aggregate_sessions(records);
            let windows = aggregate_windows(records, &sessions, WindowType::OneHour, promo);
            format_sl_window_table(&windows, opts, "1h Window", "Est 5h Budg")
        }
        SlViewMode::Window5h => {
            let sessions = aggregate_sessions(records);
            let windows = aggregate_windows(records, &sessions, WindowType::FiveHour, promo);
            format_sl_window_table(&windows, opts, "5h Window", "Est 5h Budg")
        }
        SlViewMode::Window1w => {
            let sessions = aggregate_sessions(records);
            let windows = aggregate_windows(records, &sessions, WindowType::OneWeek, promo);
            format_sl_window_table(&windows, opts, "1w Window", "Est 1w Budg")
        }
    }
}

/// Generate structured table data (headers, rows, totals) for any sl view.
/// Used for markdown, html, tsv output where we need raw data, not rendered box-drawing.
fn generate_sl_table_data(
    view_mode: &SlViewMode,
    records: &[ccost::sl::SlRecord],
    opts: &SlFormatOptions,
    _cost_diff: bool,
    promo: bool,
    _matches: &clap::ArgMatches,
) -> (Vec<String>, Vec<Vec<String>>, Option<Vec<String>>) {
    // Generate the txt table (no color) and parse it back into structured data
    // This is the simplest approach that reuses all existing table logic.
    //
    // The txt table format uses box-drawing characters:
    // ┌─┬─┐  (top)
    // │ │ │  (header row)
    // ├─┼─┤  (separator)
    // │ │ │  (data rows, separated by ├─┼─┤)
    // └─┴─┘  (bottom)
    let no_color_opts = SlFormatOptions {
        tz: opts.tz.clone(),
        price_mode: opts.price_mode,
        compact: opts.compact,
        color: false,
    };
    let table_str = generate_sl_table_content(view_mode, records, &no_color_opts, false, promo, _matches);

    // Parse box-drawing table into headers, rows, totals
    let mut headers: Vec<String> = Vec::new();
    let mut rows: Vec<Vec<String>> = Vec::new();
    let mut is_header = true;

    for line in table_str.lines() {
        // Skip border lines (start with ┌, ├, └)
        let first = line.chars().next().unwrap_or(' ');
        if first == '\u{250C}' || first == '\u{251C}' || first == '\u{2514}' {
            continue;
        }
        // Data line starts with │
        if first == '\u{2502}' {
            let cells: Vec<String> = line.split('\u{2502}')
                .filter(|s| !s.is_empty())
                .map(|s| s.trim().to_string())
                .collect();
            if is_header {
                headers = cells;
                is_header = false;
            } else {
                rows.push(cells);
            }
        }
    }

    // Last row is TOTAL if it starts with "TOTAL"
    let totals = if let Some(last) = rows.last() {
        if last.first().map(|s| s.starts_with("TOTAL")).unwrap_or(false) {
            let t = rows.pop().unwrap();
            Some(t)
        } else {
            None
        }
    } else {
        None
    };

    // Expand abbreviated headers for output formats
    let headers = headers.into_iter().map(|h| match h.as_str() {
        "Sess" => "Sessions".to_string(),
        "Segs" => "Segments".to_string(),
        "Est 5h Budg" => "Est 5h Budget".to_string(),
        "Est 1w Budg" => "Est 1w Budget".to_string(),
        _ => h,
    }).collect();

    (headers, rows, totals)
}

fn compute_cost_diffs(
    sessions: &[ccost::sl::SlSessionSummary],
    matches: &clap::ArgMatches,
) -> Vec<SlCostDiff> {
    // Load conversation JSONL via the existing pipeline
    let live_pricing = matches.get_flag("live-pricing");
    let pricing_data_path = matches.get_one::<String>("pricing-data").cloned();

    let pricing = if live_pricing {
        eprintln!("Fetching live pricing from LiteLLM...");
        match fetch_live_pricing() {
            Ok(p) => p,
            Err(e) => {
                eprintln!("Warning: could not fetch live pricing: {}", e);
                load_pricing()
            }
        }
    } else if let Some(ref path) = pricing_data_path {
        match load_pricing_from_file(path) {
            Ok(p) => p,
            Err(e) => {
                eprintln!("Warning: could not load pricing from '{}': {}", path, e);
                load_pricing()
            }
        }
    } else {
        load_pricing()
    };

    let load_opts = LoadOptions {
        claude_dir: matches.get_one::<String>("claude-dir").cloned(),
        from: None,
        to: None,
        tz: matches.get_one::<String>("tz").cloned(),
        project: None,
        model: None,
        session: None,
    };

    let load_result = load_records(&load_opts);
    let priced_records = calculate_cost(&load_result.records, Some(&pricing));

    // Sum cost per session_id from conversation JSONL
    let mut cost_by_session: HashMap<String, f64> = HashMap::new();
    for rec in &priced_records {
        *cost_by_session.entry(rec.session_id.clone()).or_default() += rec.total_cost;
    }

    // Match against sl session summaries
    sessions
        .iter()
        .map(|s| {
            // Try exact match first, then prefix match
            let litellm_cost = cost_by_session
                .get(&s.session_id)
                .copied()
                .or_else(|| {
                    // Prefix match: find conversation session whose ID starts with the sl session_id
                    cost_by_session
                        .iter()
                        .find(|(k, _)| k.starts_with(&s.session_id) || s.session_id.starts_with(k.as_str()))
                        .map(|(_, v)| *v)
                });

            let (diff, diff_pct) = match litellm_cost {
                Some(lc) => {
                    let d = s.total_cost - lc;
                    let pct = if lc.abs() > 1e-9 {
                        Some(d / lc * 100.0)
                    } else {
                        None
                    };
                    (Some(d), pct)
                }
                None => (None, None),
            };

            SlCostDiff {
                session_id: s.session_id.clone(),
                sl_cost: s.total_cost,
                litellm_cost,
                diff,
                diff_pct,
            }
        })
        .collect()
}

