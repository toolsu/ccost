// Aggregator implementation
use std::collections::{BTreeMap, HashSet};
use chrono::{DateTime, Duration, Local, TimeZone, Utc};

use super::types::*;

// ─── Timezone helpers (mirrors parser.rs) ────────────────────────────────────

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

fn format_date_in_tz(dt: &DateTime<Utc>, tz: &ResolvedTz) -> String {
    let fmt = "%Y-%m-%d";
    match tz {
        ResolvedTz::Local => dt.with_timezone(&Local).format(fmt).to_string(),
        ResolvedTz::Utc => dt.format(fmt).to_string(),
        ResolvedTz::Fixed(off) => dt.with_timezone(off).format(fmt).to_string(),
        ResolvedTz::Iana(tz) => dt.with_timezone(tz).format(fmt).to_string(),
    }
}

// ─── Segment detection helpers ────────────────────────────────────────────────

/// Returns true if the transition from `prev` to `curr` is a reset boundary.
fn is_reset(prev_cost: f64, prev_dur: u64, curr_cost: f64, curr_dur: u64) -> bool {
    // Only consider it a reset if prev values were non-zero (avoid treating 0->0 as reset)
    let prev_nonzero = prev_cost > 0.0001 || prev_dur > 100;
    if !prev_nonzero {
        return false;
    }
    // Cost dropped
    if curr_cost < prev_cost - 0.0001 {
        return true;
    }
    // Duration dropped
    if curr_dur + 100 < prev_dur {
        return true;
    }
    false
}

/// Given a sorted slice of records (all from the same session), compute the
/// segment-aware totals: sum of MAX per cumulative field across segments.
fn compute_segment_totals(
    records: &[&SlRecord],
) -> (u32, f64, u64, u64, u64, u64) {
    // Returns (segments, cost, duration, api_duration, lines_added, lines_removed)
    if records.is_empty() {
        return (0, 0.0, 0, 0, 0, 0);
    }

    let mut segments: u32 = 1;
    // Segment accumulators (running max for current segment)
    let mut seg_cost: f64 = 0.0;
    let mut seg_dur: u64 = 0;
    let mut seg_api_dur: u64 = 0;
    let mut seg_added: u64 = 0;
    let mut seg_removed: u64 = 0;

    // Running totals (sum across completed segments)
    let mut total_cost: f64 = 0.0;
    let mut total_dur: u64 = 0;
    let mut total_api_dur: u64 = 0;
    let mut total_added: u64 = 0;
    let mut total_removed: u64 = 0;

    for (i, rec) in records.iter().enumerate() {
        if i > 0 {
            let prev = records[i - 1];
            if is_reset(prev.cost_usd, prev.duration_ms, rec.cost_usd, rec.duration_ms) {
                // Flush current segment into totals
                total_cost += seg_cost;
                total_dur += seg_dur;
                total_api_dur += seg_api_dur;
                total_added += seg_added;
                total_removed += seg_removed;
                // Reset segment accumulators
                seg_cost = 0.0;
                seg_dur = 0;
                seg_api_dur = 0;
                seg_added = 0;
                seg_removed = 0;
                segments += 1;
            }
        }
        // Keep max within segment
        if rec.cost_usd > seg_cost { seg_cost = rec.cost_usd; }
        if rec.duration_ms > seg_dur { seg_dur = rec.duration_ms; }
        if rec.api_duration_ms > seg_api_dur { seg_api_dur = rec.api_duration_ms; }
        if rec.lines_added > seg_added { seg_added = rec.lines_added; }
        if rec.lines_removed > seg_removed { seg_removed = rec.lines_removed; }
    }

    // Flush final segment
    total_cost += seg_cost;
    total_dur += seg_dur;
    total_api_dur += seg_api_dur;
    total_added += seg_added;
    total_removed += seg_removed;

    (segments, total_cost, total_dur, total_api_dur, total_added, total_removed)
}

// ─── Window type enum ─────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowType {
    FiveHour,
    OneWeek,
}

// ─── Public aggregation functions ─────────────────────────────────────────────

/// Group records by session_id and compute segment-aware summaries.
pub fn aggregate_sessions(records: &[SlRecord]) -> Vec<SlSessionSummary> {
    // Group records by session_id, preserving insertion order via BTreeMap
    let mut by_session: BTreeMap<String, Vec<&SlRecord>> = BTreeMap::new();
    for rec in records {
        by_session.entry(rec.session_id.clone()).or_default().push(rec);
    }

    let mut result = Vec::new();
    for (session_id, mut recs) in by_session {
        // Sort by timestamp
        recs.sort_by_key(|r| r.ts);

        let first = recs[0];
        let last = recs[recs.len() - 1];

        let max_context_pct = recs.iter().filter_map(|r| r.context_pct).max();

        let (segments, total_cost, total_dur, total_api_dur, total_added, total_removed) =
            compute_segment_totals(&recs);

        result.push(SlSessionSummary {
            session_id,
            project: first.project.clone(),
            model_name: last.model_name.clone(),
            version: last.version.clone(),
            segments,
            total_cost,
            total_duration_ms: total_dur,
            total_api_duration_ms: total_api_dur,
            total_lines_added: total_added,
            total_lines_removed: total_removed,
            max_context_pct,
            first_ts: first.ts,
            last_ts: last.ts,
            last_five_hour_pct: last.five_hour_pct,
            last_seven_day_pct: last.seven_day_pct,
        });
    }

    result
}

/// Filter and deduplicate rate-limit entries.
/// Only keeps records where all four rate-limit fields are present,
/// and removes consecutive records with the same (five_hour_pct, seven_day_pct).
pub fn aggregate_ratelimit(records: &[SlRecord]) -> Vec<SlRateLimitEntry> {
    let mut result = Vec::new();
    let mut last_pair: Option<(u8, u8)> = None;

    for rec in records {
        // All four rate-limit fields must be Some
        let (fh_pct, fh_resets, sd_pct, sd_resets) = match (
            rec.five_hour_pct,
            rec.five_hour_resets_at,
            rec.seven_day_pct,
            rec.seven_day_resets_at,
        ) {
            (Some(a), Some(b), Some(c), Some(d)) => (a, b, c, d),
            _ => continue,
        };

        let pair = (fh_pct, sd_pct);
        if last_pair == Some(pair) {
            // Same as previous — skip
            continue;
        }
        last_pair = Some(pair);

        result.push(SlRateLimitEntry {
            ts: rec.ts,
            session_id: rec.session_id.clone(),
            five_hour_pct: fh_pct,
            five_hour_resets_at: fh_resets,
            seven_day_pct: sd_pct,
            seven_day_resets_at: sd_resets,
        });
    }

    result
}

/// Group records by rate-limit window (5h or 1w) and compute window summaries.
pub fn aggregate_windows(records: &[SlRecord], _sessions: &[SlSessionSummary], window_type: WindowType) -> Vec<SlWindowSummary> {
    // Group records by the appropriate resets_at field
    let mut by_window: BTreeMap<i64, Vec<&SlRecord>> = BTreeMap::new();
    for rec in records {
        let resets_at = match window_type {
            WindowType::FiveHour => rec.five_hour_resets_at,
            WindowType::OneWeek => rec.seven_day_resets_at,
        };
        if let Some(resets_at) = resets_at {
            by_window.entry(resets_at.timestamp()).or_default().push(rec);
        }
    }

    let mut result = Vec::new();
    for (resets_ts, window_recs) in by_window {
        let window_end = Utc.timestamp_opt(resets_ts, 0).single().unwrap_or_default();
        let window_start = match window_type {
            WindowType::FiveHour => window_end - Duration::hours(5),
            WindowType::OneWeek => window_end - Duration::days(7),
        };

        let peak_five_hour_pct = window_recs
            .iter()
            .filter_map(|r| r.five_hour_pct)
            .max()
            .unwrap_or(0);

        // Count unique sessions in this window
        let unique_sessions: HashSet<&str> = window_recs.iter().map(|r| r.session_id.as_str()).collect();
        let session_count = unique_sessions.len() as u32;

        // Track peak 7d%
        let peak_seven_day_pct = window_recs
            .iter()
            .filter_map(|r| r.seven_day_pct)
            .max();

        // Mini segment detection per session within window
        let mut by_session_window: BTreeMap<&str, Vec<&SlRecord>> = BTreeMap::new();
        for rec in &window_recs {
            by_session_window.entry(rec.session_id.as_str()).or_default().push(rec);
        }

        let mut total_cost = 0.0_f64;
        let mut total_duration_ms = 0_u64;
        let mut total_api_duration_ms = 0_u64;
        let mut total_lines_added = 0_u64;
        let mut total_lines_removed = 0_u64;
        for (_, mut sess_recs) in by_session_window {
            sess_recs.sort_by_key(|r| r.ts);
            let (_, cost, dur, api_dur, added, removed) = compute_segment_totals(&sess_recs);
            total_cost += cost;
            total_duration_ms += dur;
            total_api_duration_ms += api_dur;
            total_lines_added += added;
            total_lines_removed += removed;
        }

        // For 5h windows, use peak_five_hour_pct; for 1w windows, use peak_seven_day_pct
        let est_budget_pct = match window_type {
            WindowType::FiveHour => peak_five_hour_pct as u16,
            WindowType::OneWeek => peak_seven_day_pct.unwrap_or(0) as u16,
        };
        let est_budget = if est_budget_pct > 0 {
            Some(total_cost * 100.0 / (est_budget_pct as f64))
        } else {
            None
        };

        result.push(SlWindowSummary {
            window_start,
            window_end,
            peak_five_hour_pct,
            sessions: session_count,
            total_cost,
            est_budget,
            total_duration_ms,
            total_api_duration_ms,
            total_lines_added,
            total_lines_removed,
            peak_seven_day_pct,
        });
    }

    result
}

/// Group session summaries by project.
pub fn aggregate_by_project(sessions: &[SlSessionSummary]) -> Vec<SlProjectSummary> {
    let mut by_project: BTreeMap<String, SlProjectSummary> = BTreeMap::new();

    for s in sessions {
        let entry = by_project.entry(s.project.clone()).or_insert(SlProjectSummary {
            project: s.project.clone(),
            total_cost: 0.0,
            total_duration_ms: 0,
            total_api_duration_ms: 0,
            session_count: 0,
            total_lines_added: 0,
            total_lines_removed: 0,
            peak_five_hour_pct: None,
            peak_seven_day_pct: None,
        });
        entry.total_cost += s.total_cost;
        entry.total_duration_ms += s.total_duration_ms;
        entry.total_api_duration_ms += s.total_api_duration_ms;
        entry.session_count += 1;
        entry.total_lines_added += s.total_lines_added;
        entry.total_lines_removed += s.total_lines_removed;
        if let Some(pct) = s.last_five_hour_pct {
            entry.peak_five_hour_pct = Some(match entry.peak_five_hour_pct {
                Some(existing) => existing.max(pct),
                None => pct,
            });
        }
        if let Some(pct) = s.last_seven_day_pct {
            entry.peak_seven_day_pct = Some(match entry.peak_seven_day_pct {
                Some(existing) => existing.max(pct),
                None => pct,
            });
        }
    }

    by_project.into_values().collect()
}

/// Group sessions by date (in given timezone) and produce day summaries.
pub fn aggregate_by_day(sessions: &[SlSessionSummary], tz: Option<&str>) -> Vec<SlDaySummary> {
    let resolved_tz = resolve_tz(tz);

    let mut by_day: BTreeMap<String, SlDaySummary> = BTreeMap::new();

    for s in sessions {
        let date = format_date_in_tz(&s.first_ts, &resolved_tz);

        let entry = by_day.entry(date.clone()).or_insert(SlDaySummary {
            date,
            total_cost: 0.0,
            session_count: 0,
            peak_five_hour_pct: None,
            peak_seven_day_pct: None,
            total_duration_ms: 0,
            total_api_duration_ms: 0,
            total_lines_added: 0,
            total_lines_removed: 0,
        });

        entry.total_cost += s.total_cost;
        entry.session_count += 1;
        entry.total_duration_ms += s.total_duration_ms;
        entry.total_api_duration_ms += s.total_api_duration_ms;
        entry.total_lines_added += s.total_lines_added;
        entry.total_lines_removed += s.total_lines_removed;

        // Update peak percentages
        if let Some(pct) = s.last_five_hour_pct {
            entry.peak_five_hour_pct = Some(match entry.peak_five_hour_pct {
                Some(existing) => existing.max(pct),
                None => pct,
            });
        }
        if let Some(pct) = s.last_seven_day_pct {
            entry.peak_seven_day_pct = Some(match entry.peak_seven_day_pct {
                Some(existing) => existing.max(pct),
                None => pct,
            });
        }
    }

    by_day.into_values().collect()
}
