// Formatter implementation for sl subcommand

use chrono::{DateTime, Local, Utc};
use serde_json;

use super::types::*;
use crate::types::PriceMode;
use crate::formatters::table::format_cost;

// ─── Public option structs ────────────────────────────────────────────────────

pub struct SlFormatOptions {
    pub tz: Option<String>,
    pub price_mode: PriceMode,
    pub compact: bool,
    pub color: bool,
}

pub struct SlJsonMeta {
    pub source: String,
    pub file: String,
    pub view: String,
    pub from: Option<String>,
    pub to: Option<String>,
    pub tz: Option<String>,
    pub generated_at: String,
}

// ─── Timezone helpers ─────────────────────────────────────────────────────────

enum ResolvedTz {
    Local,
    Utc,
    Fixed(chrono::FixedOffset),
    Iana(chrono_tz::Tz),
}

fn parse_fixed_offset(s: &str) -> Option<chrono::FixedOffset> {
    let sign = if s.starts_with('+') { 1 } else { -1 };
    let rest = &s[1..];
    let parts: Vec<&str> = rest.split(':').collect();
    if parts.len() != 2 {
        return None;
    }
    let hours: i32 = parts[0].parse().ok()?;
    let minutes: i32 = parts[1].parse().ok()?;
    let total_seconds = sign * (hours * 3600 + minutes * 60);
    chrono::FixedOffset::east_opt(total_seconds)
}

fn resolve_tz(tz: Option<&str>) -> ResolvedTz {
    match tz {
        None | Some("local") => ResolvedTz::Local,
        Some("UTC") => ResolvedTz::Utc,
        Some(s) => {
            if (s.starts_with('+') || s.starts_with('-')) && s.len() == 6 {
                if let Some(offset) = parse_fixed_offset(s) {
                    return ResolvedTz::Fixed(offset);
                }
            }
            if let Ok(tz_parsed) = s.parse::<chrono_tz::Tz>() {
                return ResolvedTz::Iana(tz_parsed);
            }
            ResolvedTz::Local
        }
    }
}

// ─── Datetime formatting helpers ─────────────────────────────────────────────

/// Format a DateTime<Utc> in the given timezone using a custom format string.
pub fn fmt_dt(dt: &DateTime<Utc>, tz: Option<&str>, fmt_str: &str) -> String {
    let resolved = resolve_tz(tz);
    match &resolved {
        ResolvedTz::Local => dt.with_timezone(&Local).format(fmt_str).to_string(),
        ResolvedTz::Utc => dt.format(fmt_str).to_string(),
        ResolvedTz::Fixed(off) => dt.with_timezone(off).format(fmt_str).to_string(),
        ResolvedTz::Iana(tz_parsed) => dt.with_timezone(tz_parsed).format(fmt_str).to_string(),
    }
}

/// Format as "YYYY-MM-DD HH:MM".
pub fn fmt_time(dt: &DateTime<Utc>, tz: Option<&str>) -> String {
    fmt_dt(dt, tz, "%Y-%m-%d %H:%M")
}

/// Format as "MM-DD HH:MM".
pub fn fmt_time_short(dt: &DateTime<Utc>, tz: Option<&str>) -> String {
    fmt_dt(dt, tz, "%m-%d %H:%M")
}

/// Format a duration in milliseconds as "Xh Ym" / "Xm Ys" / "Xs".
pub fn fmt_duration(ms: u64) -> String {
    let total_secs = ms / 1000;
    let hours = total_secs / 3600;
    let minutes = (total_secs % 3600) / 60;
    let secs = total_secs % 60;

    if hours > 0 {
        format!("{}h {}m", hours, minutes)
    } else if minutes > 0 {
        format!("{}m {}s", minutes, secs)
    } else {
        format!("{}s", secs)
    }
}

/// Shorten a project path to last 2 path components with ".../" prefix.
/// e.g. "/home/user/projects/foo/bar" → ".../foo/bar"
pub fn shorten_project(path: &str) -> String {
    let components: Vec<&str> = path
        .split('/')
        .filter(|s| !s.is_empty())
        .collect();

    if components.len() <= 2 {
        path.to_string()
    } else {
        let last_two = &components[components.len() - 2..];
        format!(".../{}", last_two.join("/"))
    }
}

// ─── Generic table renderer ───────────────────────────────────────────────────

/// Calculate display width, treating ANSI escape sequences as zero-width.
fn display_width(s: &str) -> usize {
    // Strip ANSI escape sequences for width calculation
    let mut width = 0;
    let mut in_escape = false;
    for ch in s.chars() {
        if in_escape {
            if ch == 'm' {
                in_escape = false;
            }
        } else if ch == '\x1b' {
            in_escape = true;
        } else {
            width += 1;
        }
    }
    width
}

/// Render a Unicode box-drawing table from headers and rows.
/// First column is left-aligned; remaining columns are right-aligned.
pub fn render_table(headers: &[String], rows: &[Vec<String>], color: bool) -> String {
    let num_cols = headers.len();
    if num_cols == 0 {
        return String::new();
    }

    // Calculate column widths
    let mut col_widths: Vec<usize> = headers.iter().map(|h| display_width(h)).collect();

    for row in rows {
        for (i, cell) in row.iter().enumerate() {
            if i < num_cols {
                let w = display_width(cell);
                if w > col_widths[i] {
                    col_widths[i] = w;
                }
            }
        }
    }

    let green_start = if color { "\x1b[92m" } else { "" };
    let green_end = if color { "\x1b[0m" } else { "" };

    let mut output = String::new();

    // Top border: ┌─┬─┐
    output.push('\u{250C}');
    for (i, &w) in col_widths.iter().enumerate() {
        output.push_str(&"\u{2500}".repeat(w + 2));
        if i < num_cols - 1 {
            output.push('\u{252C}');
        }
    }
    output.push('\u{2510}');
    output.push('\n');

    // Header row
    output.push('\u{2502}');
    for (i, header) in headers.iter().enumerate() {
        if i == 0 {
            // Left-aligned
            output.push(' ');
            output.push_str(header);
            output.push_str(&" ".repeat(col_widths[i] - display_width(header) + 1));
        } else {
            // Right-aligned
            output.push_str(&" ".repeat(col_widths[i] - display_width(header) + 1));
            output.push_str(header);
            output.push(' ');
        }
        output.push('\u{2502}');
    }
    output.push('\n');

    // Mid separator after header
    output.push_str(&mid_separator(&col_widths));
    output.push('\n');

    // Data rows
    for (row_idx, row) in rows.iter().enumerate() {
        output.push('\u{2502}');

        // Check if this is a "totals" row (starts with green marker internally)
        // We use color for all rows uniformly here — callers handle special styling
        for (i, cell) in row.iter().enumerate() {
            if i >= num_cols {
                break;
            }
            if i == 0 {
                // Left-aligned
                output.push(' ');
                output.push_str(green_start);
                output.push_str(cell);
                output.push_str(green_end);
                output.push_str(&" ".repeat(col_widths[i] - display_width(cell) + 1));
            } else {
                // Right-aligned
                output.push_str(&" ".repeat(col_widths[i] - display_width(cell) + 1));
                output.push_str(green_start);
                output.push_str(cell);
                output.push_str(green_end);
                output.push(' ');
            }
            output.push('\u{2502}');
        }
        output.push('\n');

        // Mid separator between rows (not after the last)
        if row_idx < rows.len() - 1 {
            output.push_str(&mid_separator(&col_widths));
            output.push('\n');
        }
    }

    // Bottom border: └─┴─┘
    output.push('\u{2514}');
    for (i, &w) in col_widths.iter().enumerate() {
        output.push_str(&"\u{2500}".repeat(w + 2));
        if i < num_cols - 1 {
            output.push('\u{2534}');
        }
    }
    output.push('\u{2518}');
    output.push('\n');

    output
}

fn mid_separator(col_widths: &[usize]) -> String {
    let mut s = String::new();
    s.push('\u{251C}');
    for (i, &w) in col_widths.iter().enumerate() {
        s.push_str(&"\u{2500}".repeat(w + 2));
        if i < col_widths.len() - 1 {
            s.push('\u{253C}');
        }
    }
    s.push('\u{2524}');
    s
}

// ─── Rate-limit table ─────────────────────────────────────────────────────────

/// Format rate-limit entries as a table.
pub fn format_sl_ratelimit_table(entries: &[SlRateLimitEntry], opts: &SlFormatOptions) -> String {
    let tz = opts.tz.as_deref();

    let headers: Vec<String> = if opts.compact {
        vec!["Time".to_string(), "5h%".to_string(), "1w%".to_string(), "5h Resets".to_string()]
    } else {
        vec!["Time".to_string(), "5h%".to_string(), "1w%".to_string(), "5h Resets".to_string(), "Session".to_string()]
    };

    let rows: Vec<Vec<String>> = entries
        .iter()
        .map(|e| {
            let mut row = vec![
                fmt_time_short(&e.ts, tz),
                format!("{}%", e.five_hour_pct),
                format!("{}%", e.seven_day_pct),
                fmt_time_short(&e.five_hour_resets_at, tz),
            ];
            if !opts.compact {
                // First 8 chars of session_id
                let sess = if e.session_id.len() > 8 {
                    e.session_id[..8].to_string()
                } else {
                    e.session_id.clone()
                };
                row.push(sess);
            }
            row
        })
        .collect();

    render_table(&headers, &rows, opts.color)
}

// ─── Unified table helpers ────────────────────────────────────────────────────

/// Build unified headers for any sl --per view.
///
/// Full:    [Label] | Cost | Duration | API Time | Lines +/- | [count_label] | 5h% | 1w% | [extra_header]
/// Compact: [Label] | Cost | Duration | [count_label] | 5h%
fn unified_headers(label: &str, count_label: &str, compact: bool, extra_header: Option<&str>) -> Vec<String> {
    let mut headers = vec![label.to_string(), "Cost".to_string(), "Duration".to_string()];
    if compact {
        headers.push(count_label.to_string());
        headers.push("5h%".to_string());
    } else {
        headers.push("API Time".to_string());
        headers.push("Lines +/-".to_string());
        headers.push(count_label.to_string());
        headers.push("5h%".to_string());
        headers.push("1w%".to_string());
    }
    if let Some(extra) = extra_header {
        headers.push(extra.to_string());
    }
    headers
}

/// Format a min–max percentage range.
fn fmt_pct_range(min: Option<u8>, max: Option<u8>) -> String {
    match (min, max) {
        (Some(lo), Some(hi)) if lo == hi => format!("{}%", lo),
        (Some(lo), Some(hi)) => format!("{}–{}%", lo, hi),
        _ => "\u{2014}".to_string(),
    }
}

/// Build a unified row for any sl --per view.
fn build_unified_row(
    label: String,
    cost: f64,
    duration_ms: u64,
    api_duration_ms: u64,
    lines_added: u64,
    lines_removed: u64,
    count: u32,
    min_5h: Option<u8>,
    max_5h: Option<u8>,
    min_7d: Option<u8>,
    max_7d: Option<u8>,
    price_mode: PriceMode,
    compact: bool,
    extra: Option<String>,
) -> Vec<String> {
    let cost_str = format_cost(cost, price_mode);
    let duration_str = fmt_duration(duration_ms);

    let mut row = vec![label, cost_str, duration_str];

    if compact {
        row.push(count.to_string());
        row.push(fmt_pct_range(min_5h, max_5h));
    } else {
        let api_time_str = fmt_duration(api_duration_ms);
        let lines_str = format!("+{} -{}", lines_added, lines_removed);

        row.push(api_time_str);
        row.push(lines_str);
        row.push(count.to_string());
        row.push(fmt_pct_range(min_5h, max_5h));
        row.push(fmt_pct_range(min_7d, max_7d));
    }

    if let Some(extra_val) = extra {
        row.push(extra_val);
    }

    row
}

// ─── Session table ────────────────────────────────────────────────────────────

/// Format session summaries as a table.
pub fn format_sl_session_table(sessions: &[SlSessionSummary], opts: &SlFormatOptions) -> String {
    let headers = unified_headers("Session", "Segs", opts.compact, None);

    let rows: Vec<Vec<String>> = sessions
        .iter()
        .map(|s| {
            let sess_short = if s.session_id.len() > 8 {
                s.session_id[..8].to_string()
            } else {
                s.session_id.clone()
            };

            build_unified_row(
                sess_short,
                s.total_cost,
                s.total_duration_ms,
                s.total_api_duration_ms,
                s.total_lines_added,
                s.total_lines_removed,
                s.segments,
                s.min_five_hour_pct,
                s.max_five_hour_pct,
                s.min_seven_day_pct,
                s.max_seven_day_pct,
                opts.price_mode,
                opts.compact,
                None,
            )
        })
        .collect();

    render_table(&headers, &rows, opts.color)
}

// ─── Project table ────────────────────────────────────────────────────────────

/// Format project summaries as a table.
pub fn format_sl_project_table(projects: &[SlProjectSummary], opts: &SlFormatOptions) -> String {
    let headers = unified_headers("Project", "Sessions", opts.compact, None);

    let rows: Vec<Vec<String>> = projects
        .iter()
        .map(|p| {
            build_unified_row(
                p.project.clone(),
                p.total_cost,
                p.total_duration_ms,
                p.total_api_duration_ms,
                p.total_lines_added,
                p.total_lines_removed,
                p.session_count,
                p.min_five_hour_pct,
                p.max_five_hour_pct,
                p.min_seven_day_pct,
                p.max_seven_day_pct,
                opts.price_mode,
                opts.compact,
                None,
            )
        })
        .collect();

    render_table(&headers, &rows, opts.color)
}

// ─── Day table ────────────────────────────────────────────────────────────────

/// Format day summaries as a table.
pub fn format_sl_day_table(days: &[SlDaySummary], opts: &SlFormatOptions) -> String {
    let headers = unified_headers("Date", "Sessions", opts.compact, None);

    let rows: Vec<Vec<String>> = days
        .iter()
        .map(|d| {
            build_unified_row(
                d.date.clone(),
                d.total_cost,
                d.total_duration_ms,
                d.total_api_duration_ms,
                d.total_lines_added,
                d.total_lines_removed,
                d.session_count,
                d.min_five_hour_pct,
                d.max_five_hour_pct,
                d.min_seven_day_pct,
                d.max_seven_day_pct,
                opts.price_mode,
                opts.compact,
                None,
            )
        })
        .collect();

    render_table(&headers, &rows, opts.color)
}

// ─── Window table ─────────────────────────────────────────────────────────────

/// Format window summaries as a table.
pub fn format_sl_window_table(windows: &[SlWindowSummary], opts: &SlFormatOptions, window_label: &str) -> String {
    let tz = opts.tz.as_deref();

    let headers = unified_headers(window_label, "Sessions", opts.compact, Some("Est Budget"));

    let rows: Vec<Vec<String>> = windows
        .iter()
        .map(|w| {
            let window_str = format!(
                "{} – {}",
                fmt_time_short(&w.window_start, tz),
                fmt_time_short(&w.window_end, tz)
            );
            let est_budget_str = match w.est_budget {
                Some(b) => format_cost(b, opts.price_mode),
                None => "\u{2014}".to_string(),
            };

            build_unified_row(
                window_str,
                w.total_cost,
                w.total_duration_ms,
                w.total_api_duration_ms,
                w.total_lines_added,
                w.total_lines_removed,
                w.sessions,
                Some(w.min_five_hour_pct),
                Some(w.max_five_hour_pct),
                w.min_seven_day_pct,
                w.max_seven_day_pct,
                opts.price_mode,
                opts.compact,
                Some(est_budget_str),
            )
        })
        .collect();

    render_table(&headers, &rows, opts.color)
}

// ─── Cost diff table ──────────────────────────────────────────────────────────

/// Format cost diff entries as a table.
pub fn format_sl_cost_diff_table(
    sessions: &[SlSessionSummary],
    diffs: &[SlCostDiff],
    opts: &SlFormatOptions,
) -> String {
    let headers: Vec<String> = vec![
        "Session".to_string(),
        "Project".to_string(),
        "Cost(SL)".to_string(),
        "Cost(LiteLLM)".to_string(),
        "Diff".to_string(),
        "Diff%".to_string(),
    ];

    // Build a map from session_id to project for quick lookup
    let session_project: std::collections::HashMap<&str, &str> = sessions
        .iter()
        .map(|s| (s.session_id.as_str(), s.project.as_str()))
        .collect();

    let rows: Vec<Vec<String>> = diffs
        .iter()
        .map(|d| {
            let sess_short = if d.session_id.len() > 8 {
                d.session_id[..8].to_string()
            } else {
                d.session_id.clone()
            };
            let project = session_project
                .get(d.session_id.as_str())
                .map(|p| shorten_project(p))
                .unwrap_or_default();
            let sl_cost_str = format_cost(d.sl_cost, opts.price_mode);
            let litellm_cost_str = match d.litellm_cost {
                Some(c) => format_cost(c, opts.price_mode),
                None => "\u{2014}".to_string(),
            };
            let diff_str = match d.diff {
                Some(diff) => format_cost(diff, opts.price_mode),
                None => "\u{2014}".to_string(),
            };
            let diff_pct_str = match d.diff_pct {
                Some(pct) => format!("{:.1}%", pct),
                None => "\u{2014}".to_string(),
            };
            vec![
                sess_short,
                project,
                sl_cost_str,
                litellm_cost_str,
                diff_str,
                diff_pct_str,
            ]
        })
        .collect();

    render_table(&headers, &rows, opts.color)
}

// ─── JSON formatters ──────────────────────────────────────────────────────────

fn meta_to_json(meta: &SlJsonMeta) -> serde_json::Value {
    serde_json::json!({
        "source": meta.source,
        "file": meta.file,
        "view": meta.view,
        "from": meta.from,
        "to": meta.to,
        "tz": meta.tz,
        "generatedAt": meta.generated_at,
    })
}

/// Format rate-limit entries as JSON.
pub fn format_sl_json_ratelimit(entries: &[SlRateLimitEntry], meta: &SlJsonMeta) -> String {
    let output = serde_json::json!({
        "meta": meta_to_json(meta),
        "data": entries,
    });
    serde_json::to_string_pretty(&output).unwrap_or_else(|_| "{}".to_string())
}

/// Format session summaries as JSON (includes totals).
pub fn format_sl_json_sessions(sessions: &[SlSessionSummary], meta: &SlJsonMeta) -> String {
    let total_cost: f64 = sessions.iter().map(|s| s.total_cost).sum();
    let total_duration_ms: u64 = sessions.iter().map(|s| s.total_duration_ms).sum();
    let total_api_duration_ms: u64 = sessions.iter().map(|s| s.total_api_duration_ms).sum();
    let total_lines_added: u64 = sessions.iter().map(|s| s.total_lines_added).sum();
    let total_lines_removed: u64 = sessions.iter().map(|s| s.total_lines_removed).sum();

    let totals = serde_json::json!({
        "sessionCount": sessions.len(),
        "totalCost": total_cost,
        "totalDurationMs": total_duration_ms,
        "totalApiDurationMs": total_api_duration_ms,
        "totalLinesAdded": total_lines_added,
        "totalLinesRemoved": total_lines_removed,
    });

    let output = serde_json::json!({
        "meta": meta_to_json(meta),
        "data": sessions,
        "totals": totals,
    });
    serde_json::to_string_pretty(&output).unwrap_or_else(|_| "{}".to_string())
}

/// Format window summaries as JSON.
pub fn format_sl_json_windows(windows: &[SlWindowSummary], meta: &SlJsonMeta) -> String {
    let output = serde_json::json!({
        "meta": meta_to_json(meta),
        "data": windows,
    });
    serde_json::to_string_pretty(&output).unwrap_or_else(|_| "{}".to_string())
}

/// Format project summaries as JSON.
pub fn format_sl_json_projects(projects: &[SlProjectSummary], meta: &SlJsonMeta) -> String {
    let output = serde_json::json!({
        "meta": meta_to_json(meta),
        "data": projects,
    });
    serde_json::to_string_pretty(&output).unwrap_or_else(|_| "{}".to_string())
}

/// Format day summaries as JSON.
pub fn format_sl_json_days(days: &[SlDaySummary], meta: &SlJsonMeta) -> String {
    let output = serde_json::json!({
        "meta": meta_to_json(meta),
        "data": days,
    });
    serde_json::to_string_pretty(&output).unwrap_or_else(|_| "{}".to_string())
}

// ─── CSV formatters ───────────────────────────────────────────────────────────

fn csv_escape(field: &str) -> String {
    if field.contains(',') || field.contains('"') || field.contains('\n') || field.contains('\r') {
        let escaped = field.replace('"', "\"\"");
        format!("\"{}\"", escaped)
    } else {
        field.to_string()
    }
}

fn csv_row(fields: &[String]) -> String {
    fields.iter().map(|f| csv_escape(f)).collect::<Vec<_>>().join(",")
}

/// Format rate-limit entries as CSV.
pub fn format_sl_csv_ratelimit(entries: &[SlRateLimitEntry], tz: Option<&str>) -> String {
    let mut output = String::new();

    // Header
    output.push_str("Time,5h%,1w%,5h Resets,1w Resets,Session\n");

    for e in entries {
        let row = csv_row(&[
            fmt_time(&e.ts, tz),
            format!("{}", e.five_hour_pct),
            format!("{}", e.seven_day_pct),
            fmt_time(&e.five_hour_resets_at, tz),
            fmt_time(&e.seven_day_resets_at, tz),
            e.session_id.clone(),
        ]);
        output.push_str(&row);
        output.push('\n');
    }

    output
}

/// Format session summaries as CSV.
pub fn format_sl_csv_sessions(sessions: &[SlSessionSummary], opts: &SlFormatOptions) -> String {
    let mut output = String::new();

    // Header
    output.push_str("Session,Project,Cost,Duration,API Time,Lines Added,Lines Removed,Ctx%,Segments\n");

    for s in sessions {
        let ctx_pct = match s.max_context_pct {
            Some(p) => p.to_string(),
            None => String::new(),
        };
        let row = csv_row(&[
            s.session_id.clone(),
            s.project.clone(),
            format!("{:.6}", s.total_cost),
            fmt_duration(s.total_duration_ms),
            fmt_duration(s.total_api_duration_ms),
            s.total_lines_added.to_string(),
            s.total_lines_removed.to_string(),
            ctx_pct,
            s.segments.to_string(),
        ]);
        let _ = opts; // price_mode not used in raw CSV numeric output
        output.push_str(&row);
        output.push('\n');
    }

    output
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{TimeZone, Utc};

    fn make_ratelimit_entry(
        ts_secs: i64,
        session_id: &str,
        five_hour_pct: u8,
        five_hour_resets_secs: i64,
        seven_day_pct: u8,
        seven_day_resets_secs: i64,
    ) -> SlRateLimitEntry {
        SlRateLimitEntry {
            ts: Utc.timestamp_opt(ts_secs, 0).single().unwrap(),
            session_id: session_id.to_string(),
            five_hour_pct,
            five_hour_resets_at: Utc.timestamp_opt(five_hour_resets_secs, 0).single().unwrap(),
            seven_day_pct,
            seven_day_resets_at: Utc.timestamp_opt(seven_day_resets_secs, 0).single().unwrap(),
        }
    }

    fn make_session_summary(
        session_id: &str,
        project: &str,
        total_cost: f64,
        total_duration_ms: u64,
        total_api_duration_ms: u64,
        total_lines_added: u64,
        total_lines_removed: u64,
        max_context_pct: Option<u8>,
        segments: u32,
    ) -> SlSessionSummary {
        SlSessionSummary {
            session_id: session_id.to_string(),
            project: project.to_string(),
            model_name: "Claude Sonnet".to_string(),
            version: "1.0.0".to_string(),
            segments,
            total_cost,
            total_duration_ms,
            total_api_duration_ms,
            total_lines_added,
            total_lines_removed,
            max_context_pct,
            first_ts: Utc.timestamp_opt(1_774_483_200, 0).single().unwrap(),
            last_ts: Utc.timestamp_opt(1_774_483_200 + 3600, 0).single().unwrap(),
            min_five_hour_pct: Some(30),
            max_five_hour_pct: Some(30),
            min_seven_day_pct: Some(50),
            max_seven_day_pct: Some(50),
        }
    }

    #[test]
    fn test_fmt_duration_seconds() {
        assert_eq!(fmt_duration(0), "0s");
        assert_eq!(fmt_duration(5000), "5s");
        assert_eq!(fmt_duration(59000), "59s");
    }

    #[test]
    fn test_fmt_duration_minutes() {
        assert_eq!(fmt_duration(60_000), "1m 0s");
        assert_eq!(fmt_duration(90_000), "1m 30s");
        assert_eq!(fmt_duration(3599_000), "59m 59s");
    }

    #[test]
    fn test_fmt_duration_hours() {
        assert_eq!(fmt_duration(3600_000), "1h 0m");
        assert_eq!(fmt_duration(3660_000), "1h 1m");
        assert_eq!(fmt_duration(7200_000), "2h 0m");
        assert_eq!(fmt_duration(7320_000), "2h 2m");
    }

    #[test]
    fn test_shorten_project_long() {
        assert_eq!(shorten_project("/home/user/projects/foo/bar"), ".../foo/bar");
        assert_eq!(shorten_project("/a/b/c/d"), ".../c/d");
    }

    #[test]
    fn test_shorten_project_short() {
        assert_eq!(shorten_project("/foo/bar"), "/foo/bar");
        assert_eq!(shorten_project("/foo"), "/foo");
        assert_eq!(shorten_project("foo"), "foo");
    }

    #[test]
    fn test_fmt_time_utc() {
        // 2026-03-26T12:00:00Z
        let dt = Utc.timestamp_opt(1_774_526_400, 0).single().unwrap();
        let result = fmt_time(&dt, Some("UTC"));
        assert_eq!(result, "2026-03-26 12:00");
    }

    #[test]
    fn test_fmt_time_short_utc() {
        let dt = Utc.timestamp_opt(1_774_526_400, 0).single().unwrap();
        let result = fmt_time_short(&dt, Some("UTC"));
        assert_eq!(result, "03-26 12:00");
    }

    #[test]
    fn test_ratelimit_table_headers() {
        let entries = vec![make_ratelimit_entry(
            1_774_483_200, "session-abc123", 30, 1_774_500_000, 50, 1_775_000_000
        )];
        let opts = SlFormatOptions {
            tz: Some("UTC".to_string()),
            price_mode: PriceMode::Decimal,
            compact: false,
            color: false,
        };
        let result = format_sl_ratelimit_table(&entries, &opts);
        assert!(result.contains("Time"), "should contain Time header");
        assert!(result.contains("5h%"), "should contain 5h% header");
        assert!(result.contains("1w%"), "should contain 1w% header");
        assert!(result.contains("5h Resets"), "should contain 5h Resets header");
        assert!(result.contains("Session"), "should contain Session header");
    }

    #[test]
    fn test_ratelimit_table_compact_no_session() {
        let entries = vec![make_ratelimit_entry(
            1_774_483_200, "session-abc123", 30, 1_774_500_000, 50, 1_775_000_000
        )];
        let opts = SlFormatOptions {
            tz: Some("UTC".to_string()),
            price_mode: PriceMode::Decimal,
            compact: true,
            color: false,
        };
        let result = format_sl_ratelimit_table(&entries, &opts);
        assert!(result.contains("5h%"), "should contain 5h% header");
        assert!(!result.contains("Session"), "compact should hide Session column");
    }

    #[test]
    fn test_ratelimit_table_values() {
        let entries = vec![make_ratelimit_entry(
            1_774_483_200, "session-abc123", 45, 1_774_500_000, 72, 1_775_000_000
        )];
        let opts = SlFormatOptions {
            tz: Some("UTC".to_string()),
            price_mode: PriceMode::Decimal,
            compact: false,
            color: false,
        };
        let result = format_sl_ratelimit_table(&entries, &opts);
        assert!(result.contains("45%"), "should contain 45%");
        assert!(result.contains("72%"), "should contain 72%");
        // Session truncated to 8 chars: "session-"
        assert!(result.contains("session-"), "should contain first 8 chars of session_id");
    }

    #[test]
    fn test_session_table_full_headers() {
        let sessions = vec![make_session_summary(
            "abc123", "/home/user/foo/bar", 0.50, 3600_000, 1800_000, 100, 50, Some(75), 2
        )];
        let opts = SlFormatOptions {
            tz: Some("UTC".to_string()),
            price_mode: PriceMode::Decimal,
            compact: false,
            color: false,
        };
        let result = format_sl_session_table(&sessions, &opts);
        assert!(result.contains("Session"), "should contain Session header");
        assert!(result.contains("Cost"), "should contain Cost header");
        assert!(result.contains("Duration"), "should contain Duration header");
        assert!(result.contains("API Time"), "should contain API Time header");
        assert!(result.contains("Lines +/-"), "should contain Lines +/- header");
        assert!(result.contains("Segs"), "should contain Segs header");
        assert!(result.contains("5h%"), "should contain 5h% header");
        assert!(result.contains("1w%"), "should contain 1w% header");
    }

    #[test]
    fn test_session_table_compact_headers() {
        let sessions = vec![make_session_summary(
            "abc123", "/home/user/foo/bar", 0.50, 3600_000, 1800_000, 100, 50, Some(75), 2
        )];
        let opts = SlFormatOptions {
            tz: Some("UTC".to_string()),
            price_mode: PriceMode::Decimal,
            compact: true,
            color: false,
        };
        let result = format_sl_session_table(&sessions, &opts);
        assert!(result.contains("Session"), "should contain Session header");
        assert!(result.contains("Cost"), "should contain Cost header");
        assert!(result.contains("Segs"), "should contain Segs header");
        assert!(result.contains("5h%"), "should contain 5h% header");
        assert!(!result.contains("API Time"), "compact should not contain API Time");
        assert!(!result.contains("1w%"), "compact should not contain 1w%");
    }

    #[test]
    fn test_json_ratelimit_structure() {
        let entries = vec![make_ratelimit_entry(
            1_774_483_200, "session-abc123", 30, 1_774_500_000, 50, 1_775_000_000
        )];
        let meta = SlJsonMeta {
            source: "test".to_string(),
            file: "test.jsonl".to_string(),
            view: "ratelimit".to_string(),
            from: None,
            to: None,
            tz: Some("UTC".to_string()),
            generated_at: "2026-03-26T00:00:00Z".to_string(),
        };
        let result = format_sl_json_ratelimit(&entries, &meta);
        let parsed: serde_json::Value = serde_json::from_str(&result).expect("should be valid JSON");
        assert!(parsed["meta"].is_object(), "should have meta object");
        assert!(parsed["data"].is_array(), "should have data array");
        assert_eq!(parsed["data"].as_array().unwrap().len(), 1);
        assert_eq!(parsed["meta"]["view"], "ratelimit");
    }

    #[test]
    fn test_json_sessions_has_totals() {
        let sessions = vec![
            make_session_summary("s1", "/proj/a", 0.5, 3600_000, 1800_000, 10, 5, None, 1),
            make_session_summary("s2", "/proj/b", 0.3, 1800_000, 900_000, 5, 2, None, 1),
        ];
        let meta = SlJsonMeta {
            source: "test".to_string(),
            file: "test.jsonl".to_string(),
            view: "sessions".to_string(),
            from: None,
            to: None,
            tz: None,
            generated_at: "2026-03-26T00:00:00Z".to_string(),
        };
        let result = format_sl_json_sessions(&sessions, &meta);
        let parsed: serde_json::Value = serde_json::from_str(&result).expect("valid JSON");
        assert!(parsed["totals"].is_object(), "should have totals object");
        assert_eq!(parsed["totals"]["sessionCount"], 2);
        let total_cost = parsed["totals"]["totalCost"].as_f64().unwrap();
        assert!((total_cost - 0.8).abs() < 1e-9, "total_cost={}", total_cost);
    }

    #[test]
    fn test_json_windows_structure() {
        use chrono::TimeZone;
        let windows = vec![SlWindowSummary {
            window_start: Utc.timestamp_opt(1_774_483_200, 0).single().unwrap(),
            window_end: Utc.timestamp_opt(1_774_500_000, 0).single().unwrap(),
            min_five_hour_pct: 45,
            max_five_hour_pct: 45,
            sessions: 3,
            total_cost: 1.23,
            est_budget: Some(2.73),
            total_duration_ms: 5000,
            total_api_duration_ms: 2000,
            total_lines_added: 10,
            total_lines_removed: 5,
            min_seven_day_pct: Some(60),
            max_seven_day_pct: Some(60),
        }];
        let meta = SlJsonMeta {
            source: "test".to_string(),
            file: "test.jsonl".to_string(),
            view: "windows".to_string(),
            from: None,
            to: None,
            tz: None,
            generated_at: "2026-03-26T00:00:00Z".to_string(),
        };
        let result = format_sl_json_windows(&windows, &meta);
        let parsed: serde_json::Value = serde_json::from_str(&result).expect("valid JSON");
        assert!(parsed["data"].is_array());
        assert_eq!(parsed["data"][0]["minFiveHourPct"], 45);
        assert_eq!(parsed["data"][0]["maxFiveHourPct"], 45);
    }

    #[test]
    fn test_csv_ratelimit_headers() {
        let entries = vec![make_ratelimit_entry(
            1_774_483_200, "session-abc123", 30, 1_774_500_000, 50, 1_775_000_000
        )];
        let result = format_sl_csv_ratelimit(&entries, Some("UTC"));
        let first_line = result.lines().next().unwrap();
        assert_eq!(first_line, "Time,5h%,1w%,5h Resets,1w Resets,Session");
    }

    #[test]
    fn test_csv_ratelimit_values() {
        let entries = vec![make_ratelimit_entry(
            1_774_483_200, "session-abc123", 30, 1_774_500_000, 50, 1_775_000_000
        )];
        let result = format_sl_csv_ratelimit(&entries, Some("UTC"));
        let lines: Vec<&str> = result.lines().collect();
        assert_eq!(lines.len(), 2); // header + 1 data row
        assert!(lines[1].contains("30"), "should contain 5h%");
        assert!(lines[1].contains("50"), "should contain 1w%");
        assert!(lines[1].contains("session-abc123"), "should contain full session id");
    }

    #[test]
    fn test_csv_sessions_headers() {
        let sessions = vec![make_session_summary(
            "abc123", "/proj/a", 0.5, 3600_000, 1800_000, 10, 5, Some(50), 1
        )];
        let opts = SlFormatOptions {
            tz: Some("UTC".to_string()),
            price_mode: PriceMode::Decimal,
            compact: false,
            color: false,
        };
        let result = format_sl_csv_sessions(&sessions, &opts);
        let first_line = result.lines().next().unwrap();
        assert!(first_line.contains("Session"), "header should contain Session");
        assert!(first_line.contains("Cost"), "header should contain Cost");
        assert!(first_line.contains("API Time"), "header should contain API Time");
    }
}
