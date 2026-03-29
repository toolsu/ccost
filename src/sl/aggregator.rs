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
