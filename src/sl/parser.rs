use chrono::{DateTime, Local, TimeZone, Utc};
use serde::Deserialize;
use std::fs;

use super::types::{SlLoadOptions, SlRecord};

// ─── Raw deserialization structs ───────────────────────────────────────────

#[derive(Deserialize)]
struct RawRateLimitWindow {
    used_percentage: Option<f64>,
    resets_at: Option<i64>,
}

#[derive(Deserialize)]
struct RawRateLimits {
    five_hour: Option<RawRateLimitWindow>,
    seven_day: Option<RawRateLimitWindow>,
}

#[derive(Deserialize)]
struct RawContextWindow {
    used_percentage: Option<f64>,
    context_window_size: u64,
}

#[derive(Deserialize)]
struct RawCost {
    total_cost_usd: f64,
    total_duration_ms: u64,
    total_api_duration_ms: u64,
    #[serde(default)]
    total_lines_added: u64,
    #[serde(default)]
    total_lines_removed: u64,
}

#[derive(Deserialize)]
struct RawModel {
    id: String,
    #[serde(default)]
    display_name: String,
}

#[derive(Deserialize)]
struct RawWorkspace {
    project_dir: String,
}

#[derive(Deserialize)]
struct RawData {
    session_id: String,
    workspace: RawWorkspace,
    model: RawModel,
    #[serde(default)]
    version: String,
    cost: RawCost,
    context_window: RawContextWindow,
    #[serde(default)]
    rate_limits: Option<RawRateLimits>,
}

#[derive(Deserialize)]
struct RawEntry {
    ts: i64,
    data: RawData,
}

// ─── Timezone resolution (mirrors src/parser.rs) ───────────────────────────

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

/// Format a UTC datetime as "YYYY-MM-DDTHH:MM:SS" in the given resolved timezone.
fn format_in_tz(dt: &DateTime<Utc>, tz: &ResolvedTz) -> String {
    let fmt = "%Y-%m-%dT%H:%M:%S";
    match tz {
        ResolvedTz::Local => dt.with_timezone(&Local).format(fmt).to_string(),
        ResolvedTz::Utc => dt.format(fmt).to_string(),
        ResolvedTz::Fixed(off) => dt.with_timezone(off).format(fmt).to_string(),
        ResolvedTz::Iana(tz) => dt.with_timezone(tz).format(fmt).to_string(),
    }
}

fn get_date_part(s: &str) -> &str {
    if s.len() >= 10 {
        &s[..10]
    } else {
        s
    }
}

// ─── Public API ────────────────────────────────────────────────────────────

/// Load `SlRecord`s from a JSONL file, applying filters from `opts`.
///
/// Returns `(records, skipped_count)` where `skipped_count` is the number of
/// malformed lines that could not be parsed.
pub fn load_sl_records(file_path: &str, opts: &SlLoadOptions) -> (Vec<SlRecord>, usize) {
    let content = match fs::read_to_string(file_path) {
        Ok(c) => c,
        Err(_) => return (Vec::new(), 0),
    };

    // Resolve timezone once for efficient repeated use
    let resolved_tz = resolve_tz(opts.tz.as_deref());

    // Normalize and precompute filter values
    let from_normalized = opts.from.as_ref().map(|s| s.replace(' ', "T"));
    let to_normalized = opts.to.as_ref().map(|s| s.replace(' ', "T"));
    let from_is_date_only = from_normalized
        .as_ref()
        .map(|s| s.len() == 10)
        .unwrap_or(false);
    let to_is_date_only = to_normalized
        .as_ref()
        .map(|s| s.len() == 10)
        .unwrap_or(false);
    let needs_date_filter = from_normalized.is_some() || to_normalized.is_some();

    let session_lower = opts.session.as_ref().map(|s| s.to_lowercase());
    let project_lower = opts.project.as_ref().map(|s| s.to_lowercase());
    let model_lower = opts.model.as_ref().map(|s| s.to_lowercase());

    let mut records: Vec<SlRecord> = Vec::new();
    let mut skipped: usize = 0;

    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        let entry: RawEntry = match serde_json::from_str(line) {
            Ok(e) => e,
            Err(_) => {
                skipped += 1;
                continue;
            }
        };

        // Convert unix timestamp (seconds) to DateTime<Utc>
        let ts = match Utc.timestamp_opt(entry.ts, 0).single() {
            Some(t) => t,
            None => {
                skipped += 1;
                continue;
            }
        };

        // from/to filter
        if needs_date_filter {
            let formatted = format_in_tz(&ts, &resolved_tz);
            if let Some(ref from_val) = from_normalized {
                let cmp = if from_is_date_only {
                    get_date_part(&formatted)
                } else {
                    &formatted
                };
                if cmp < from_val.as_str() {
                    continue;
                }
            }
            if let Some(ref to_val) = to_normalized {
                let cmp = if to_is_date_only {
                    get_date_part(&formatted)
                } else {
                    &formatted
                };
                if cmp > to_val.as_str() {
                    continue;
                }
            }
        }

        let d = &entry.data;

        // session filter
        if let Some(ref f) = session_lower {
            if !d.session_id.to_lowercase().contains(f.as_str()) {
                continue;
            }
        }

        // project filter
        if let Some(ref f) = project_lower {
            if !d.workspace.project_dir.to_lowercase().contains(f.as_str()) {
                continue;
            }
        }

        // model filter (model_id + model_name combined)
        if let Some(ref f) = model_lower {
            let combined = format!("{} {}", d.model.id, d.model.display_name).to_lowercase();
            if !combined.contains(f.as_str()) {
                continue;
            }
        }

        // Rate limits (optional)
        let (five_hour_pct, five_hour_resets_at, seven_day_pct, seven_day_resets_at) =
            match &d.rate_limits {
                Some(rl) => {
                    let fh_pct = rl
                        .five_hour
                        .as_ref()
                        .and_then(|w| w.used_percentage)
                        .map(|v| v.round() as u8);
                    let fh_resets = rl
                        .five_hour
                        .as_ref()
                        .and_then(|w| w.resets_at)
                        .and_then(|secs| Utc.timestamp_opt(secs, 0).single());
                    let sd_pct = rl
                        .seven_day
                        .as_ref()
                        .and_then(|w| w.used_percentage)
                        .map(|v| v.round() as u8);
                    let sd_resets = rl
                        .seven_day
                        .as_ref()
                        .and_then(|w| w.resets_at)
                        .and_then(|secs| Utc.timestamp_opt(secs, 0).single());
                    (fh_pct, fh_resets, sd_pct, sd_resets)
                }
                None => (None, None, None, None),
            };

        records.push(SlRecord {
            ts,
            session_id: d.session_id.clone(),
            project: d.workspace.project_dir.clone(),
            model_id: d.model.id.clone(),
            model_name: d.model.display_name.clone(),
            version: d.version.clone(),
            cost_usd: d.cost.total_cost_usd,
            duration_ms: d.cost.total_duration_ms,
            api_duration_ms: d.cost.total_api_duration_ms,
            lines_added: d.cost.total_lines_added,
            lines_removed: d.cost.total_lines_removed,
            context_pct: d.context_window.used_percentage.map(|v| v.round() as u8),
            context_size: d.context_window.context_window_size,
            five_hour_pct,
            five_hour_resets_at,
            seven_day_pct,
            seven_day_resets_at,
        });
    }

    (records, skipped)
}
