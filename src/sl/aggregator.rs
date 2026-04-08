// Aggregator implementation
use chrono::{DateTime, Duration, Local, TimeZone, Utc};
use std::collections::{BTreeMap, HashSet};

use super::types::*;
use crate::utils::parse_fixed_offset;

// ─── Timezone helpers (mirrors parser.rs) ────────────────────────────────────

enum ResolvedTz {
    Local,
    Utc,
    Fixed(chrono::FixedOffset),
    Iana(chrono_tz::Tz),
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

fn format_datetime_in_tz(dt: &DateTime<Utc>, tz: &ResolvedTz) -> String {
    let fmt = "%Y-%m-%dT%H:%M:%S";
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
fn compute_segment_totals(records: &[&SlRecord]) -> (u32, f64, u64, u64, u64, u64) {
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
            if is_reset(
                prev.cost_usd,
                prev.duration_ms,
                rec.cost_usd,
                rec.duration_ms,
            ) {
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
        if rec.cost_usd > seg_cost {
            seg_cost = rec.cost_usd;
        }
        if rec.duration_ms > seg_dur {
            seg_dur = rec.duration_ms;
        }
        if rec.api_duration_ms > seg_api_dur {
            seg_api_dur = rec.api_duration_ms;
        }
        if rec.lines_added > seg_added {
            seg_added = rec.lines_added;
        }
        if rec.lines_removed > seg_removed {
            seg_removed = rec.lines_removed;
        }
    }

    // Flush final segment
    total_cost += seg_cost;
    total_dur += seg_dur;
    total_api_dur += seg_api_dur;
    total_added += seg_added;
    total_removed += seg_removed;

    (
        segments,
        total_cost,
        total_dur,
        total_api_dur,
        total_added,
        total_removed,
    )
}

// ─── Window type enum ─────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowType {
    OneHour,
    FiveHour,
    OneWeek,
}

// ─── Promo intervals (2x usage) ─────────────────────────────────────────────

/// 2025-12 and 2026-03 Claude double-usage promo intervals (UTC timestamps, start inclusive, end exclusive).
const PROMO_INTERVALS: &[(i64, i64)] = &[
    (1766620800, 1767225600),
    (1773374400, 1773403200),
    (1773424800, 1773662400),
    (1773684000, 1773748800),
    (1773770400, 1773835200),
    (1773856800, 1773921600),
    (1773943200, 1774008000),
    (1774029600, 1774267200),
    (1774288800, 1774353600),
    (1774375200, 1774440000),
    (1774461600, 1774526400),
    (1774548000, 1774612800),
    (1774634400, 1774756800),
];

/// Compute the fraction of time range [start, end) that overlaps with promo intervals.
pub fn promo_overlap_ratio(start_ts: i64, end_ts: i64) -> f64 {
    let window_dur = end_ts - start_ts;
    if window_dur <= 0 {
        return 0.0;
    }
    let mut overlap = 0_i64;
    for &(ps, pe) in PROMO_INTERVALS {
        let os = start_ts.max(ps);
        let oe = end_ts.min(pe);
        if oe > os {
            overlap += oe - os;
        }
    }
    overlap as f64 / window_dur as f64
}

/// Compute promo-adjusted est_budget. During promo, delta_pct represents half the
/// normal rate (budget is doubled), so we scale the delta up to get the normal budget.
fn compute_est_budget(
    total_cost: f64,
    delta_pct: u16,
    promo: bool,
    window_start_ts: i64,
    window_end_ts: i64,
) -> Option<f64> {
    if delta_pct == 0 {
        return None;
    }
    let adjustment = if promo {
        let ratio = promo_overlap_ratio(window_start_ts, window_end_ts);
        1.0 + ratio // ranges from 1.0 (no promo) to 2.0 (full promo)
    } else {
        1.0
    };
    let adjusted_delta = delta_pct as f64 * adjustment;
    if adjusted_delta > 0.0 {
        Some(total_cost * 100.0 / adjusted_delta)
    } else {
        None
    }
}

// ─── Public aggregation functions ─────────────────────────────────────────────

/// Group records by session_id and compute segment-aware summaries.
///
/// Subtracts each session's first record values as a baseline to handle
/// continued sessions (`claude --continue`) where cumulative counters
/// carry over from the predecessor session without resetting.
pub fn aggregate_sessions(records: &[SlRecord]) -> Vec<SlSessionSummary> {
    // Group records by session_id, preserving insertion order via BTreeMap
    let mut by_session: BTreeMap<String, Vec<&SlRecord>> = BTreeMap::new();
    for rec in records {
        by_session
            .entry(rec.session_id.clone())
            .or_default()
            .push(rec);
    }

    let mut result = Vec::new();
    for (session_id, mut recs) in by_session {
        // Sort by timestamp
        recs.sort_by_key(|r| r.ts);

        let first = recs[0];
        let last = recs[recs.len() - 1];

        let max_context_pct = recs.iter().filter_map(|r| r.context_pct).max();

        let (segments, raw_cost, raw_dur, raw_api_dur, raw_added, raw_removed) =
            compute_segment_totals(&recs);

        // Subtract session baseline (first record's cumulative values).
        // For continued sessions, this removes inherited costs from predecessor.
        // For fresh sessions starting at ~0, this has negligible effect.
        let total_cost = (raw_cost - first.cost_usd).max(0.0);
        let total_dur = raw_dur.saturating_sub(first.duration_ms);
        let total_api_dur = raw_api_dur.saturating_sub(first.api_duration_ms);
        let total_added = raw_added.saturating_sub(first.lines_added);
        let total_removed = raw_removed.saturating_sub(first.lines_removed);

        let five_hour_pcts: Vec<u8> = recs.iter().filter_map(|r| r.five_hour_pct).collect();
        let seven_day_pcts: Vec<u8> = recs.iter().filter_map(|r| r.seven_day_pct).collect();

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
            min_five_hour_pct: five_hour_pcts.iter().copied().min(),
            max_five_hour_pct: five_hour_pcts.iter().copied().max(),
            min_seven_day_pct: seven_day_pcts.iter().copied().min(),
            max_seven_day_pct: seven_day_pcts.iter().copied().max(),
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
    // Pre-populate with each session's first cost as baseline.
    // For continued sessions, this prevents the inherited cost from appearing
    // as a large delta on the first action.
    let mut last_cost_by_session: BTreeMap<&str, f64> = BTreeMap::new();
    for rec in records {
        last_cost_by_session
            .entry(rec.session_id.as_str())
            .or_insert(rec.cost_usd);
    }

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
            // Same as previous — skip entry but do NOT update cost baseline,
            // so the next displayed action accumulates the skipped cost delta.
            continue;
        }
        last_pair = Some(pair);

        // Compute cost delta
        let prev_cost = last_cost_by_session
            .get(rec.session_id.as_str())
            .copied()
            .unwrap_or(0.0);
        let cost_delta = if rec.cost_usd < prev_cost {
            rec.cost_usd // segment reset
        } else {
            rec.cost_usd - prev_cost
        };
        last_cost_by_session.insert(rec.session_id.as_str(), rec.cost_usd);

        result.push(SlRateLimitEntry {
            ts: rec.ts,
            session_id: rec.session_id.clone(),
            cost_delta,
            five_hour_pct: fh_pct,
            five_hour_resets_at: fh_resets,
            seven_day_pct: sd_pct,
            seven_day_resets_at: sd_resets,
        });
    }

    // Flush remaining unaccounted cost at EOF for each session.
    // When a same-pct run ends at EOF/session boundary, the accumulated
    // cost delta from skipped records is lost — there's no "next emitted
    // entry" to capture it. Fix: add remaining delta to each session's
    // last emitted entry.

    // Build map of last entry index per session (owned keys to avoid borrow conflict).
    let mut last_entry_idx: std::collections::HashMap<String, usize> =
        std::collections::HashMap::new();
    for (i, entry) in result.iter().enumerate() {
        last_entry_idx.insert(entry.session_id.clone(), i);
    }

    // Track final cost from qualifying records (ones with all 4 rate-limit fields).
    let mut final_cost: std::collections::HashMap<&str, f64> = std::collections::HashMap::new();
    for rec in records {
        if rec.five_hour_pct.is_some()
            && rec.five_hour_resets_at.is_some()
            && rec.seven_day_pct.is_some()
            && rec.seven_day_resets_at.is_some()
        {
            final_cost.insert(rec.session_id.as_str(), rec.cost_usd);
        }
    }

    // Track last qualifying record per session (for creating synthetic entries if needed).
    let mut last_qualifying: std::collections::HashMap<&str, &SlRecord> =
        std::collections::HashMap::new();
    for rec in records {
        if rec.five_hour_pct.is_some()
            && rec.five_hour_resets_at.is_some()
            && rec.seven_day_pct.is_some()
            && rec.seven_day_resets_at.is_some()
        {
            last_qualifying.insert(rec.session_id.as_str(), rec);
        }
    }

    for (sid, &fc) in &final_cost {
        let baseline = last_cost_by_session.get(sid).copied().unwrap_or(0.0);
        let remaining = if fc < baseline {
            fc // segment reset at end
        } else {
            fc - baseline
        };
        if remaining > 1e-9 {
            if let Some(&idx) = last_entry_idx.get(*sid) {
                result[idx].cost_delta += remaining;
            } else if let Some(rec) = last_qualifying.get(sid) {
                // Session had qualifying records but none were emitted (global
                // last_pair dedup filtered them all). Emit a synthetic entry.
                result.push(SlRateLimitEntry {
                    ts: rec.ts,
                    session_id: rec.session_id.clone(),
                    cost_delta: remaining,
                    five_hour_pct: rec.five_hour_pct.unwrap(),
                    five_hour_resets_at: rec.five_hour_resets_at.unwrap(),
                    seven_day_pct: rec.seven_day_pct.unwrap(),
                    seven_day_resets_at: rec.seven_day_resets_at.unwrap(),
                });
            }
        }
    }

    result
}

/// Compute segment-aware totals for a session's records up to (but not including) a given time.
///
/// When no records qualify (all are after `before`), returns the session baseline
/// instead of zeros, so that delta computation correctly excludes inherited costs
/// from continued sessions.
fn segment_totals_before(
    session_recs: &[&SlRecord], // must be sorted by ts
    before: DateTime<Utc>,
    baseline: (f64, u64, u64, u64, u64),
) -> (f64, u64, u64, u64, u64) {
    let filtered: Vec<&SlRecord> = session_recs
        .iter()
        .filter(|r| r.ts < before)
        .copied()
        .collect();
    if filtered.is_empty() {
        return baseline;
    }
    let (_, cost, dur, api, added, removed) = compute_segment_totals(&filtered);
    (cost, dur, api, added, removed)
}

/// Compute a single window summary.
/// `window_recs`: records belonging to this rate-limit window (filtered by resets_at) — for pct, session count.
/// `by_session`: ALL records per session, sorted by ts — for delta-based cost/duration computation.
fn build_window_summary(
    window_recs: &[&SlRecord],
    by_session: &BTreeMap<&str, Vec<&SlRecord>>,
    window_start: DateTime<Utc>,
    window_end: DateTime<Utc>,
    window_type: WindowType,
    promo: bool,
    five_hour_resets_at: Option<DateTime<Utc>>,
) -> Option<SlWindowSummary> {
    if window_recs.is_empty() {
        return None;
    }

    // pct and session count from window_recs (correctly grouped by resets_at)
    let five_hour_pcts: Vec<u8> = window_recs.iter().filter_map(|r| r.five_hour_pct).collect();
    let max_five_hour_pct = five_hour_pcts.iter().copied().max().unwrap_or(0);
    let min_five_hour_pct = five_hour_pcts.iter().copied().min().unwrap_or(0);

    let seven_day_pcts: Vec<u8> = window_recs.iter().filter_map(|r| r.seven_day_pct).collect();
    let max_seven_day_pct = seven_day_pcts.iter().copied().max();
    let min_seven_day_pct = seven_day_pcts.iter().copied().min();

    let session_ids: HashSet<&str> = window_recs.iter().map(|r| r.session_id.as_str()).collect();
    let session_count = session_ids.len() as u32;

    // Compute delta totals per session: totals_up_to(end) - totals_up_to(start)
    let mut total_cost = 0.0_f64;
    let mut total_duration_ms = 0_u64;
    let mut total_api_duration_ms = 0_u64;
    let mut total_lines_added = 0_u64;
    let mut total_lines_removed = 0_u64;
    for sid in &session_ids {
        if let Some(sess_recs) = by_session.get(sid) {
            // Session baseline: first record's cumulative values (may be inherited from continue)
            let first = sess_recs[0];
            let baseline = (
                first.cost_usd,
                first.duration_ms,
                first.api_duration_ms,
                first.lines_added,
                first.lines_removed,
            );
            let (c_end, d_end, a_end, la_end, lr_end) =
                segment_totals_before(sess_recs, window_end, baseline);
            let (c_start, d_start, a_start, la_start, lr_start) =
                segment_totals_before(sess_recs, window_start, baseline);
            total_cost += (c_end - c_start).max(0.0);
            total_duration_ms += d_end.saturating_sub(d_start);
            total_api_duration_ms += a_end.saturating_sub(a_start);
            total_lines_added += la_end.saturating_sub(la_start);
            total_lines_removed += lr_end.saturating_sub(lr_start);
        }
    }

    let delta_pct = match window_type {
        WindowType::OneHour | WindowType::FiveHour => {
            max_five_hour_pct.saturating_sub(min_five_hour_pct) as u16
        }
        WindowType::OneWeek => {
            let max = max_seven_day_pct.unwrap_or(0);
            let min = min_seven_day_pct.unwrap_or(0);
            max.saturating_sub(min) as u16
        }
    };
    let est_budget = compute_est_budget(
        total_cost,
        delta_pct,
        promo,
        window_start.timestamp(),
        window_end.timestamp(),
    );

    let (est_5h_budget, est_1w_budget) = match window_type {
        WindowType::OneHour | WindowType::FiveHour => (est_budget, None),
        WindowType::OneWeek => (None, est_budget),
    };

    Some(SlWindowSummary {
        window_start,
        window_end,
        min_five_hour_pct,
        max_five_hour_pct,
        sessions: session_count,
        total_cost,
        est_5h_budget,
        est_1w_budget,
        total_duration_ms,
        total_api_duration_ms,
        total_lines_added,
        total_lines_removed,
        min_seven_day_pct,
        max_seven_day_pct,
        five_hour_resets_at,
    })
}

/// Group records by rate-limit window (1h, 5h, or 1w) and compute window summaries.
pub fn aggregate_windows(
    records: &[SlRecord],
    _sessions: &[SlSessionSummary],
    window_type: WindowType,
    promo: bool,
) -> Vec<SlWindowSummary> {
    // Pre-group ALL records by session_id, sorted by ts (for delta computation)
    let mut by_session: BTreeMap<&str, Vec<&SlRecord>> = BTreeMap::new();
    for rec in records {
        by_session
            .entry(rec.session_id.as_str())
            .or_default()
            .push(rec);
    }
    for recs in by_session.values_mut() {
        recs.sort_by_key(|r| r.ts);
    }

    // Group records by resets_at (for pct and session counting)
    let group_key = match window_type {
        WindowType::OneHour | WindowType::FiveHour => WindowType::FiveHour,
        WindowType::OneWeek => WindowType::OneWeek,
    };

    let mut by_window: BTreeMap<i64, Vec<&SlRecord>> = BTreeMap::new();
    for rec in records {
        let resets_at = match group_key {
            WindowType::FiveHour => rec.five_hour_resets_at,
            WindowType::OneWeek => rec.seven_day_resets_at,
            _ => unreachable!(),
        };
        if let Some(r) = resets_at {
            by_window.entry(r.timestamp()).or_default().push(rec);
        }
    }

    let mut result = Vec::new();
    for (resets_ts, window_recs) in &by_window {
        let window_end = Utc
            .timestamp_opt(*resets_ts, 0)
            .single()
            .unwrap_or_default();
        let parent_duration = match group_key {
            WindowType::FiveHour => Duration::hours(5),
            WindowType::OneWeek => Duration::days(7),
            _ => unreachable!(),
        };
        let window_start = window_end - parent_duration;

        if window_type == WindowType::OneHour {
            // Split by timestamp within the resets_at-grouped records
            let fh_resets_at = Some(window_end);
            for i in 0..5 {
                let chunk_start = window_start + Duration::hours(i);
                let chunk_end = window_start + Duration::hours(i + 1);
                let chunk_recs: Vec<&SlRecord> = window_recs
                    .iter()
                    .filter(|r| r.ts >= chunk_start && r.ts < chunk_end)
                    .copied()
                    .collect();
                if let Some(summary) = build_window_summary(
                    &chunk_recs,
                    &by_session,
                    chunk_start,
                    chunk_end,
                    window_type,
                    promo,
                    fh_resets_at,
                ) {
                    result.push(summary);
                }
            }
        } else {
            if let Some(summary) = build_window_summary(
                window_recs,
                &by_session,
                window_start,
                window_end,
                window_type,
                promo,
                None,
            ) {
                result.push(summary);
            }
        }
    }

    result
}

/// Filter windows to only those overlapping with [from, to] range.
///
/// A window is kept if its time range overlaps with the filter range:
/// - window_end > from (window doesn't end at or before range start)
/// - window_start < to (window doesn't start at or after range end)
///
/// The from/to strings are compared in the given timezone, matching the
/// format used by `load_sl_records` for record-level filtering.
pub fn filter_windows_by_range(
    windows: Vec<SlWindowSummary>,
    from: &Option<String>,
    to: &Option<String>,
    tz: Option<&str>,
) -> Vec<SlWindowSummary> {
    if from.is_none() && to.is_none() {
        return windows;
    }
    let resolved_tz = resolve_tz(tz);
    let from_normalized = from.as_ref().map(|s| {
        let n = s.replace(' ', "T");
        if n.len() == 16 {
            format!("{}:00", n)
        } else {
            n
        }
    });
    let to_normalized = to.as_ref().map(|s| {
        let n = s.replace(' ', "T");
        if n.len() == 16 {
            format!("{}:00", n)
        } else {
            n
        }
    });

    windows
        .into_iter()
        .filter(|w| {
            if let Some(ref from_val) = from_normalized {
                let end_str = format_datetime_in_tz(&w.window_end, &resolved_tz);
                if end_str.as_str() <= from_val.as_str() {
                    return false;
                }
            }
            if let Some(ref to_val) = to_normalized {
                let start_str = format_datetime_in_tz(&w.window_start, &resolved_tz);
                if start_str.as_str() >= to_val.as_str() {
                    return false;
                }
            }
            true
        })
        .collect()
}

/// Group session summaries by project.
pub fn aggregate_by_project(sessions: &[SlSessionSummary]) -> Vec<SlProjectSummary> {
    let mut by_project: BTreeMap<String, SlProjectSummary> = BTreeMap::new();

    for s in sessions {
        let entry = by_project
            .entry(s.project.clone())
            .or_insert(SlProjectSummary {
                project: s.project.clone(),
                total_cost: 0.0,
                total_duration_ms: 0,
                total_api_duration_ms: 0,
                session_count: 0,
                total_lines_added: 0,
                total_lines_removed: 0,
                min_five_hour_pct: None,
                max_five_hour_pct: None,
                min_seven_day_pct: None,
                max_seven_day_pct: None,
            });
        entry.total_cost += s.total_cost;
        entry.total_duration_ms += s.total_duration_ms;
        entry.total_api_duration_ms += s.total_api_duration_ms;
        entry.session_count += 1;
        entry.total_lines_added += s.total_lines_added;
        entry.total_lines_removed += s.total_lines_removed;
        if let Some(pct) = s.min_five_hour_pct {
            entry.min_five_hour_pct = Some(match entry.min_five_hour_pct {
                Some(existing) => existing.min(pct),
                None => pct,
            });
        }
        if let Some(pct) = s.max_five_hour_pct {
            entry.max_five_hour_pct = Some(match entry.max_five_hour_pct {
                Some(existing) => existing.max(pct),
                None => pct,
            });
        }
        if let Some(pct) = s.min_seven_day_pct {
            entry.min_seven_day_pct = Some(match entry.min_seven_day_pct {
                Some(existing) => existing.min(pct),
                None => pct,
            });
        }
        if let Some(pct) = s.max_seven_day_pct {
            entry.max_seven_day_pct = Some(match entry.max_seven_day_pct {
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
            min_five_hour_pct: None,
            max_five_hour_pct: None,
            min_seven_day_pct: None,
            max_seven_day_pct: None,
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

        if let Some(pct) = s.min_five_hour_pct {
            entry.min_five_hour_pct = Some(match entry.min_five_hour_pct {
                Some(existing) => existing.min(pct),
                None => pct,
            });
        }
        if let Some(pct) = s.max_five_hour_pct {
            entry.max_five_hour_pct = Some(match entry.max_five_hour_pct {
                Some(existing) => existing.max(pct),
                None => pct,
            });
        }
        if let Some(pct) = s.min_seven_day_pct {
            entry.min_seven_day_pct = Some(match entry.min_seven_day_pct {
                Some(existing) => existing.min(pct),
                None => pct,
            });
        }
        if let Some(pct) = s.max_seven_day_pct {
            entry.max_seven_day_pct = Some(match entry.max_seven_day_pct {
                Some(existing) => existing.max(pct),
                None => pct,
            });
        }
    }

    by_day.into_values().collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    // ─── SlRecord test helper ─────────────────────────────────────────────────

    fn make_record(
        ts_secs: i64,
        session_id: &str,
        project: &str,
        cost_usd: f64,
        duration_ms: u64,
        five_hour_pct: Option<u8>,
        five_hour_resets_at: Option<i64>,
        seven_day_pct: Option<u8>,
        seven_day_resets_at: Option<i64>,
    ) -> SlRecord {
        SlRecord {
            ts: Utc.timestamp_opt(ts_secs, 0).unwrap(),
            session_id: session_id.to_string(),
            project: project.to_string(),
            model_id: "claude-3-5-sonnet".to_string(),
            model_name: "Claude 3.5 Sonnet".to_string(),
            version: "1.0".to_string(),
            cost_usd,
            duration_ms,
            api_duration_ms: 0,
            lines_added: 0,
            lines_removed: 0,
            context_pct: None,
            context_size: 0,
            five_hour_pct,
            five_hour_resets_at: five_hour_resets_at.map(|s| Utc.timestamp_opt(s, 0).unwrap()),
            seven_day_pct,
            seven_day_resets_at: seven_day_resets_at.map(|s| Utc.timestamp_opt(s, 0).unwrap()),
        }
    }

    fn make_session_summary(
        session_id: &str,
        project: &str,
        first_ts_secs: i64,
        total_cost: f64,
        min_five_hour_pct: Option<u8>,
        max_five_hour_pct: Option<u8>,
        min_seven_day_pct: Option<u8>,
        max_seven_day_pct: Option<u8>,
    ) -> SlSessionSummary {
        let first_ts = Utc.timestamp_opt(first_ts_secs, 0).unwrap();
        SlSessionSummary {
            session_id: session_id.to_string(),
            project: project.to_string(),
            model_name: "Claude 3.5 Sonnet".to_string(),
            version: "1.0".to_string(),
            segments: 1,
            total_cost,
            total_duration_ms: 1000,
            total_api_duration_ms: 500,
            total_lines_added: 10,
            total_lines_removed: 5,
            max_context_pct: None,
            first_ts,
            last_ts: first_ts,
            min_five_hour_pct,
            max_five_hour_pct,
            min_seven_day_pct,
            max_seven_day_pct,
        }
    }

    // ─── promo_overlap_ratio ─────────────────────────────────────────────────

    #[test]
    fn test_promo_overlap_ratio_no_overlap() {
        // Range completely before first promo interval (1766620800, 1767225600)
        let ratio = promo_overlap_ratio(1000000000, 1000001000);
        assert_eq!(ratio, 0.0);
    }

    #[test]
    fn test_promo_overlap_ratio_full_overlap() {
        // Range that exactly matches the first promo interval
        let ps = 1766620800_i64;
        let pe = 1767225600_i64;
        let ratio = promo_overlap_ratio(ps, pe);
        assert!((ratio - 1.0).abs() < 1e-9);
    }

    #[test]
    fn test_promo_overlap_ratio_partial() {
        // Range that covers exactly half of the first promo interval
        let ps = 1766620800_i64;
        let pe = 1767225600_i64;
        let mid = (ps + pe) / 2;
        // Window [ps, mid) is exactly 50% of [ps, pe)
        let ratio = promo_overlap_ratio(ps, mid);
        assert!(
            (ratio - 1.0).abs() < 1e-9,
            "ratio should be 1.0 (window is entirely within promo): {ratio}"
        );

        // Window [mid, pe) – another 50% chunk, also fully within promo
        let ratio2 = promo_overlap_ratio(mid, pe);
        assert!((ratio2 - 1.0).abs() < 1e-9, "ratio2: {ratio2}");

        // Window twice as large: [ps, pe + (pe-ps)] — promo covers only half
        let double_end = pe + (pe - ps);
        let ratio3 = promo_overlap_ratio(ps, double_end);
        assert!(
            (ratio3 - 0.5).abs() < 1e-6,
            "ratio3 should be ~0.5: {ratio3}"
        );
    }

    #[test]
    fn test_promo_overlap_ratio_zero_window() {
        // start == end → window_dur == 0 → should return 0.0 (no division)
        let ts = 1766620800_i64;
        let ratio = promo_overlap_ratio(ts, ts);
        assert_eq!(ratio, 0.0);
    }

    // ─── compute_est_budget ──────────────────────────────────────────────────

    #[test]
    fn test_compute_est_budget_no_promo() {
        // promo=false, delta_pct=50, cost=10.0 → 10.0 * 100 / 50 = 20.0
        let result = compute_est_budget(10.0, 50, false, 0, 1000);
        assert_eq!(result, Some(20.0));
    }

    #[test]
    fn test_compute_est_budget_with_promo() {
        // promo=true, window fully within first promo interval → ratio=1.0 → adjustment=2.0
        // delta_pct=50, cost=10.0 → 10.0 * 100 / (50 * 2.0) = 10.0
        let ps = 1766620800_i64;
        let pe = 1767225600_i64;
        let result = compute_est_budget(10.0, 50, true, ps, pe);
        // ratio=1.0, adjusted_delta = 50 * 2.0 = 100.0, est = 10.0 * 100 / 100 = 10.0
        assert!(result.is_some());
        let val = result.unwrap();
        assert!((val - 10.0).abs() < 1e-9, "expected ~10.0, got {val}");
    }

    #[test]
    fn test_compute_est_budget_zero_delta() {
        // delta_pct=0 → None
        let result = compute_est_budget(10.0, 0, false, 0, 1000);
        assert!(result.is_none());
    }

    // ─── is_reset ────────────────────────────────────────────────────────────

    #[test]
    fn test_is_reset_cost_drop() {
        // cost drops significantly → true
        assert!(is_reset(5.0, 1000, 0.5, 1000));
    }

    #[test]
    fn test_is_reset_duration_drop() {
        // duration drops by more than 100 ms → true
        assert!(is_reset(5.0, 5000, 5.0, 100));
    }

    #[test]
    fn test_is_reset_prev_zero() {
        // prev values near zero → not a reset (new session just started)
        assert!(!is_reset(0.0, 0, 0.0, 0));
        assert!(!is_reset(0.00005, 50, 0.0, 0));
    }

    #[test]
    fn test_is_reset_increasing() {
        // all values increasing → not a reset
        assert!(!is_reset(1.0, 1000, 2.0, 2000));
    }

    // ─── aggregate_ratelimit – EOF flush ────────────────────────────────────

    #[test]
    fn test_aggregate_ratelimit_eof_flush_remaining() {
        // Session A: two records with the same (5h%, 7d%) pair.
        // The first is emitted; the second is skipped (same pair).
        // At EOF, the cost delta from the skipped second record should be flushed
        // onto the first (and only emitted) entry.
        let resets_5h = 1700000000_i64 + 5 * 3600;
        let resets_7d = 1700000000_i64 + 7 * 86400;

        let rec1 = make_record(
            1700000000,
            "sessA",
            "/proj",
            1.0, // baseline cost
            1000,
            Some(10),
            Some(resets_5h),
            Some(5),
            Some(resets_7d),
        );
        let rec2 = make_record(
            1700001000,
            "sessA",
            "/proj",
            2.5, // cost grew by 1.5 since rec1
            2000,
            Some(10), // same pct pair — will be skipped
            Some(resets_5h),
            Some(5),
            Some(resets_7d),
        );

        let records = vec![rec1, rec2];
        let entries = aggregate_ratelimit(&records);

        // Only one entry should be emitted (second was deduped by same-pair rule)
        assert_eq!(entries.len(), 1);

        // The emitted entry's cost_delta should include the skipped record's cost
        // Baseline for sessA is initialised to rec1.cost_usd = 1.0 (first record).
        // rec1 emitted: delta = 1.0 - 1.0 = 0.0 (first record sets baseline)
        // rec2 skipped: cost grew to 2.5
        // EOF flush: remaining = 2.5 - 1.0 = 1.5 added to entry
        assert!(
            entries[0].cost_delta > 0.0,
            "cost_delta should be > 0 after EOF flush, got {}",
            entries[0].cost_delta
        );
    }

    #[test]
    fn test_aggregate_ratelimit_eof_flush_synthetic_entry() {
        // All records in a session have the same pct pair → nothing is emitted normally.
        // EOF flush should create a synthetic entry rather than losing the cost.
        let resets_5h = 1700000000_i64 + 5 * 3600;
        let resets_7d = 1700000000_i64 + 7 * 86400;

        // Make a *different* session emit something first to populate last_pair,
        // then sessB whose entries all share pct=(10,5) — they will all be skipped
        // because last_pair (from sessA below) starts as None, so sessB's first
        // record IS emitted. Let's make a simpler scenario instead:
        // Two sessions: sessA emits first (pair 10,5), sessB also has pair (10,5)
        // so sessB's records are ALL skipped → synthetic entry must be created.

        let rec_a = make_record(
            1700000000,
            "sessA",
            "/proj",
            0.5,
            500,
            Some(10),
            Some(resets_5h),
            Some(5),
            Some(resets_7d),
        );
        // sessB records with same pct pair — will be deduped since last_pair already (10,5)
        let rec_b1 = make_record(
            1700000100,
            "sessB",
            "/proj",
            1.0,
            1000,
            Some(10),
            Some(resets_5h),
            Some(5),
            Some(resets_7d),
        );
        let rec_b2 = make_record(
            1700000200,
            "sessB",
            "/proj",
            3.0,
            1500,
            Some(10),
            Some(resets_5h),
            Some(5),
            Some(resets_7d),
        );

        let records = vec![rec_a, rec_b1, rec_b2];
        let entries = aggregate_ratelimit(&records);

        // sessA emitted 1 entry; sessB had all entries skipped → synthetic entry added
        assert!(
            entries.len() >= 2,
            "expected at least 2 entries (sessA + synthetic for sessB), got {}",
            entries.len()
        );

        let sessb_entry = entries.iter().find(|e| e.session_id == "sessB");
        assert!(
            sessb_entry.is_some(),
            "expected a synthetic entry for sessB"
        );
        let sessb_entry = sessb_entry.unwrap();
        // sessB cost went from baseline 1.0 (first record) to 3.0 → delta = 2.0
        assert!(
            sessb_entry.cost_delta > 0.0,
            "sessB synthetic entry cost_delta should be > 0, got {}",
            sessb_entry.cost_delta
        );
    }

    // ─── aggregate_by_project – pct merging ─────────────────────────────────

    #[test]
    fn test_aggregate_by_project_pct_merging() {
        // Two sessions in the same project with different pct ranges
        let s1 = make_session_summary(
            "sess1",
            "/myproject",
            1700000000,
            1.0,
            Some(10),
            Some(30),
            Some(2),
            Some(8),
        );
        let s2 = make_session_summary(
            "sess2",
            "/myproject",
            1700001000,
            2.0,
            Some(5),
            Some(40),
            Some(1),
            Some(12),
        );

        let projects = aggregate_by_project(&[s1, s2]);
        assert_eq!(projects.len(), 1);

        let p = &projects[0];
        assert_eq!(p.project, "/myproject");
        assert!((p.total_cost - 3.0).abs() < 1e-9);
        assert_eq!(p.session_count, 2);
        // min_five_hour_pct should be the minimum of (10, 5) = 5
        assert_eq!(p.min_five_hour_pct, Some(5));
        // max_five_hour_pct should be the maximum of (30, 40) = 40
        assert_eq!(p.max_five_hour_pct, Some(40));
        // min_seven_day_pct: min(2, 1) = 1
        assert_eq!(p.min_seven_day_pct, Some(1));
        // max_seven_day_pct: max(8, 12) = 12
        assert_eq!(p.max_seven_day_pct, Some(12));
    }

    #[test]
    fn test_aggregate_by_project_multiple_projects() {
        let s1 = make_session_summary("sess1", "projA", 1700000000, 1.5, None, None, None, None);
        let s2 = make_session_summary("sess2", "projB", 1700001000, 2.5, None, None, None, None);
        let s3 = make_session_summary("sess3", "projA", 1700002000, 0.5, None, None, None, None);

        let mut projects = aggregate_by_project(&[s1, s2, s3]);
        projects.sort_by(|a, b| a.project.cmp(&b.project));

        assert_eq!(projects.len(), 2);
        let pa = projects.iter().find(|p| p.project == "projA").unwrap();
        let pb = projects.iter().find(|p| p.project == "projB").unwrap();

        assert!((pa.total_cost - 2.0).abs() < 1e-9);
        assert_eq!(pa.session_count, 2);
        assert!((pb.total_cost - 2.5).abs() < 1e-9);
        assert_eq!(pb.session_count, 1);
    }

    // ─── aggregate_by_day ────────────────────────────────────────────────────

    #[test]
    fn test_aggregate_by_day_utc() {
        // Sessions on 2025-01-15 and 2025-01-16 UTC
        let s1 = make_session_summary(
            "sess1", "/proj", // 2025-01-15T10:00:00Z
            1736935200, 1.0, None, None, None, None,
        );
        let s2 = make_session_summary(
            "sess2", "/proj", // 2025-01-16T10:00:00Z
            1737021600, 2.0, None, None, None, None,
        );
        let s3 = make_session_summary(
            "sess3", "/proj", // 2025-01-15T23:00:00Z – same day as s1
            1736982000, 0.5, None, None, None, None,
        );

        let mut days = aggregate_by_day(&[s1, s2, s3], Some("UTC"));
        days.sort_by(|a, b| a.date.cmp(&b.date));

        assert_eq!(days.len(), 2);
        assert_eq!(days[0].date, "2025-01-15");
        assert!((days[0].total_cost - 1.5).abs() < 1e-9);
        assert_eq!(days[0].session_count, 2);
        assert_eq!(days[1].date, "2025-01-16");
        assert!((days[1].total_cost - 2.0).abs() < 1e-9);
        assert_eq!(days[1].session_count, 1);
    }

    // ─── format_date_in_tz / format_datetime_in_tz ──────────────────────────

    #[test]
    fn test_format_date_in_tz_utc() {
        let dt = Utc.with_ymd_and_hms(2025, 6, 15, 23, 0, 0).unwrap();
        let tz = resolve_tz(Some("UTC"));
        assert_eq!(format_date_in_tz(&dt, &tz), "2025-06-15");
    }

    #[test]
    fn test_format_date_in_tz_fixed_offset() {
        // 2025-06-15T23:00:00Z + 02:00 → 2025-06-16T01:00:00 → date "2025-06-16"
        let dt = Utc.with_ymd_and_hms(2025, 6, 15, 23, 0, 0).unwrap();
        let tz = resolve_tz(Some("+02:00"));
        assert_eq!(format_date_in_tz(&dt, &tz), "2025-06-16");
    }

    #[test]
    fn test_format_date_in_tz_iana() {
        // 2025-06-15T23:00:00Z in Asia/Shanghai (+08:00) → 2025-06-16T07:00:00 → "2025-06-16"
        let dt = Utc.with_ymd_and_hms(2025, 6, 15, 23, 0, 0).unwrap();
        let tz = resolve_tz(Some("Asia/Shanghai"));
        assert_eq!(format_date_in_tz(&dt, &tz), "2025-06-16");
    }

    #[test]
    fn test_format_datetime_in_tz_utc() {
        let dt = Utc.with_ymd_and_hms(2025, 3, 10, 8, 30, 45).unwrap();
        let tz = resolve_tz(Some("UTC"));
        assert_eq!(format_datetime_in_tz(&dt, &tz), "2025-03-10T08:30:45");
    }

    #[test]
    fn test_format_datetime_in_tz_fixed_offset() {
        let dt = Utc.with_ymd_and_hms(2025, 3, 10, 8, 0, 0).unwrap();
        let tz = resolve_tz(Some("+05:30"));
        assert_eq!(format_datetime_in_tz(&dt, &tz), "2025-03-10T13:30:00");
    }

    #[test]
    fn test_format_datetime_in_tz_iana() {
        let dt = Utc.with_ymd_and_hms(2025, 1, 10, 8, 0, 0).unwrap();
        let tz = resolve_tz(Some("US/Eastern"));
        // January: EST is UTC-5
        assert_eq!(format_datetime_in_tz(&dt, &tz), "2025-01-10T03:00:00");
    }

    // ─── aggregate_windows – 1h splitting ───────────────────────────────────

    #[test]
    fn test_aggregate_windows_1h_splitting() {
        // All records share a single five_hour_resets_at window.
        // Records are spread across two different hours within that window.
        // aggregate_windows(WindowType::OneHour) should split them into hour-chunks.
        let resets_ts = 1700018000_i64; // some future time = window_end
        let window_end = Utc.timestamp_opt(resets_ts, 0).unwrap();
        let window_start = window_end - Duration::hours(5);

        // hour 0: records in [window_start, window_start+1h)
        let h0_ts = window_start.timestamp() + 100;
        // hour 1: records in [window_start+1h, window_start+2h)
        let h1_ts = window_start.timestamp() + 3700;

        let rec1 = make_record(
            h0_ts,
            "sessA",
            "/proj",
            1.0,
            1000,
            Some(10),
            Some(resets_ts),
            Some(3),
            Some(resets_ts + 7 * 86400),
        );
        let rec2 = make_record(
            h1_ts,
            "sessA",
            "/proj",
            2.0,
            2000,
            Some(20),
            Some(resets_ts),
            Some(4),
            Some(resets_ts + 7 * 86400),
        );

        let records = vec![rec1, rec2];
        let sessions = aggregate_sessions(&records);
        let windows = aggregate_windows(&records, &sessions, WindowType::OneHour, false);

        // Should produce 2 window summaries (one per occupied hour-chunk)
        assert_eq!(
            windows.len(),
            2,
            "expected 2 hour-chunks, got {}",
            windows.len()
        );
        // All summaries should have five_hour_resets_at set
        for w in &windows {
            assert!(w.five_hour_resets_at.is_some());
        }
    }
}
