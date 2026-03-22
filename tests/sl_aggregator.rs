use ccost::sl::types::*;
use ccost::sl::{
    aggregate_sessions, aggregate_ratelimit, aggregate_windows, aggregate_by_project,
    aggregate_by_day, WindowType,
};
use chrono::{TimeZone, Utc};

// ─── Helper to build a minimal SlRecord ──────────────────────────────────────

fn make_record(
    ts_secs: i64,
    session_id: &str,
    project: &str,
    cost_usd: f64,
    duration_ms: u64,
    api_duration_ms: u64,
    lines_added: u64,
    lines_removed: u64,
    context_pct: Option<u8>,
    five_hour_pct: Option<u8>,
    five_hour_resets_at: Option<i64>,
    seven_day_pct: Option<u8>,
    seven_day_resets_at: Option<i64>,
) -> SlRecord {
    SlRecord {
        ts: Utc.timestamp_opt(ts_secs, 0).single().unwrap(),
        session_id: session_id.to_string(),
        project: project.to_string(),
        model_id: "claude-sonnet-4-5".to_string(),
        model_name: "Claude Sonnet".to_string(),
        version: "1.0.0".to_string(),
        cost_usd,
        duration_ms,
        api_duration_ms,
        lines_added,
        lines_removed,
        context_pct,
        context_size: 200000,
        five_hour_pct,
        five_hour_resets_at: five_hour_resets_at
            .and_then(|s| Utc.timestamp_opt(s, 0).single()),
        seven_day_pct,
        seven_day_resets_at: seven_day_resets_at
            .and_then(|s| Utc.timestamp_opt(s, 0).single()),
    }
}

// ─── aggregate_sessions ───────────────────────────────────────────────────────

#[test]
fn test_single_segment_session() {
    // Three records for one session with monotonically increasing cumulative values
    let records = vec![
        make_record(1000, "s1", "/proj/a", 0.1, 500, 200, 5, 2, Some(10), None, None, None, None),
        make_record(2000, "s1", "/proj/a", 0.3, 1500, 700, 12, 5, Some(20), None, None, None, None),
        make_record(3000, "s1", "/proj/a", 0.5, 2500, 1100, 20, 9, Some(30), None, None, None, None),
    ];

    let summaries = aggregate_sessions(&records);
    assert_eq!(summaries.len(), 1);

    let s = &summaries[0];
    assert_eq!(s.session_id, "s1");
    assert_eq!(s.segments, 1);
    // Max of each cumulative field within the single segment
    assert!((s.total_cost - 0.5).abs() < 1e-9, "total_cost={}", s.total_cost);
    assert_eq!(s.total_duration_ms, 2500);
    assert_eq!(s.total_api_duration_ms, 1100);
    assert_eq!(s.total_lines_added, 20);
    assert_eq!(s.total_lines_removed, 9);
    assert_eq!(s.max_context_pct, Some(30));
    assert_eq!(s.first_ts.timestamp(), 1000);
    assert_eq!(s.last_ts.timestamp(), 3000);
}

#[test]
fn test_multi_segment_session_cost_reset() {
    // Segment 1: records at ts 1000, 2000 with cost going 0.1 -> 0.5
    // Segment 2: cost drops to 0.1 (reset), then rises to 0.3
    let records = vec![
        make_record(1000, "s1", "/proj/a", 0.1, 500, 200, 5, 2, None, None, None, None, None),
        make_record(2000, "s1", "/proj/a", 0.5, 1500, 700, 15, 5, None, None, None, None, None),
        // Reset: cost dropped significantly
        make_record(3000, "s1", "/proj/a", 0.1, 500, 200, 5, 2, None, None, None, None, None),
        make_record(4000, "s1", "/proj/a", 0.3, 1000, 400, 10, 4, None, None, None, None, None),
    ];

    let summaries = aggregate_sessions(&records);
    assert_eq!(summaries.len(), 1);

    let s = &summaries[0];
    assert_eq!(s.segments, 2, "should detect 2 segments");
    // Segment 1 max: cost=0.5, dur=1500; Segment 2 max: cost=0.3, dur=1000 → sum = 0.8, 2500
    assert!((s.total_cost - 0.8).abs() < 1e-9, "total_cost={}", s.total_cost);
    assert_eq!(s.total_duration_ms, 2500);
}

#[test]
fn test_multi_segment_session_duration_reset() {
    // Duration drops triggers a new segment
    let records = vec![
        make_record(1000, "s1", "/proj/a", 0.2, 2000, 800, 10, 5, None, None, None, None, None),
        make_record(2000, "s1", "/proj/a", 0.5, 5000, 2000, 20, 10, None, None, None, None, None),
        // Duration drops from 5000 to 200 (> 100 difference) and cost also drops
        make_record(3000, "s1", "/proj/a", 0.1, 200, 100, 5, 2, None, None, None, None, None),
        make_record(4000, "s1", "/proj/a", 0.4, 3000, 1500, 18, 8, None, None, None, None, None),
    ];

    let summaries = aggregate_sessions(&records);
    assert_eq!(summaries.len(), 1);

    let s = &summaries[0];
    assert_eq!(s.segments, 2, "duration drop should trigger new segment");
    // Seg1 max: 0.5, 5000; Seg2 max: 0.4, 3000 → total: 0.9, 8000
    assert!((s.total_cost - 0.9).abs() < 1e-9);
    assert_eq!(s.total_duration_ms, 8000);
}

#[test]
fn test_zero_zero_not_treated_as_reset() {
    // Two consecutive zero records should NOT be treated as a reset
    let records = vec![
        make_record(1000, "s1", "/proj/a", 0.0, 0, 0, 0, 0, None, None, None, None, None),
        make_record(2000, "s1", "/proj/a", 0.0, 0, 0, 0, 0, None, None, None, None, None),
        make_record(3000, "s1", "/proj/a", 0.2, 500, 200, 5, 2, None, None, None, None, None),
    ];

    let summaries = aggregate_sessions(&records);
    assert_eq!(summaries.len(), 1);
    assert_eq!(summaries[0].segments, 1, "0->0 should not be a reset");
}

#[test]
fn test_zero_nonzero_then_drop_is_reset() {
    // 0 -> 0.5 -> 0.1 (drop) should detect reset at the third record
    let records = vec![
        make_record(1000, "s1", "/proj/a", 0.0, 0, 0, 0, 0, None, None, None, None, None),
        make_record(2000, "s1", "/proj/a", 0.5, 1000, 400, 10, 5, None, None, None, None, None),
        make_record(3000, "s1", "/proj/a", 0.1, 300, 100, 3, 1, None, None, None, None, None),
    ];

    let summaries = aggregate_sessions(&records);
    assert_eq!(summaries.len(), 1);
    assert_eq!(summaries[0].segments, 2, "0.5->0.1 is a reset");
    // Seg1 max: 0.5, 1000; Seg2 max: 0.1, 300 → total: 0.6, 1300
    assert!((summaries[0].total_cost - 0.6).abs() < 1e-9);
}

#[test]
fn test_multiple_sessions() {
    let records = vec![
        make_record(1000, "s1", "/proj/a", 0.1, 500, 200, 5, 2, Some(5), None, None, None, None),
        make_record(2000, "s1", "/proj/a", 0.3, 1500, 600, 12, 5, Some(15), None, None, None, None),
        make_record(1500, "s2", "/proj/b", 0.2, 800, 300, 8, 3, Some(8), None, None, None, None),
        make_record(2500, "s2", "/proj/b", 0.6, 2000, 900, 25, 10, Some(20), None, None, None, None),
    ];

    let summaries = aggregate_sessions(&records);
    assert_eq!(summaries.len(), 2);

    let s1 = summaries.iter().find(|s| s.session_id == "s1").unwrap();
    let s2 = summaries.iter().find(|s| s.session_id == "s2").unwrap();

    assert!((s1.total_cost - 0.3).abs() < 1e-9);
    assert!((s2.total_cost - 0.6).abs() < 1e-9);
    assert_eq!(s1.max_context_pct, Some(15));
    assert_eq!(s2.max_context_pct, Some(20));
}

#[test]
fn test_last_ts_fields() {
    let records = vec![
        make_record(1000, "s1", "/proj/a", 0.1, 500, 200, 5, 2, None, Some(10), Some(9999), Some(20), Some(99999)),
        make_record(2000, "s1", "/proj/a", 0.3, 1500, 600, 12, 5, None, Some(30), Some(9999), Some(50), Some(99999)),
    ];

    let summaries = aggregate_sessions(&records);
    assert_eq!(summaries.len(), 1);
    let s = &summaries[0];
    assert_eq!(s.min_five_hour_pct, Some(10));
    assert_eq!(s.max_five_hour_pct, Some(30));
    assert_eq!(s.min_seven_day_pct, Some(20));
    assert_eq!(s.max_seven_day_pct, Some(50));
}

// ─── aggregate_ratelimit ──────────────────────────────────────────────────────

#[test]
fn test_ratelimit_requires_all_fields() {
    let records = vec![
        // Missing seven_day fields
        make_record(1000, "s1", "/proj/a", 0.1, 500, 200, 5, 2, None, Some(10), Some(9999), None, None),
        // All present
        make_record(2000, "s1", "/proj/a", 0.2, 600, 250, 6, 3, None, Some(20), Some(9999), Some(30), Some(99999)),
        // Missing five_hour_resets_at
        make_record(3000, "s1", "/proj/a", 0.3, 700, 300, 7, 4, None, Some(25), None, Some(35), Some(99999)),
    ];

    let entries = aggregate_ratelimit(&records);
    assert_eq!(entries.len(), 1, "only fully-populated records should pass");
    assert_eq!(entries[0].five_hour_pct, 20);
    assert_eq!(entries[0].seven_day_pct, 30);
}

#[test]
fn test_ratelimit_dedup_consecutive_same() {
    let records = vec![
        make_record(1000, "s1", "/proj/a", 0.1, 500, 200, 5, 2, None, Some(10), Some(9999), Some(20), Some(99999)),
        make_record(2000, "s1", "/proj/a", 0.2, 600, 250, 6, 3, None, Some(10), Some(9999), Some(20), Some(99999)),
        make_record(3000, "s1", "/proj/a", 0.3, 700, 300, 7, 4, None, Some(10), Some(9999), Some(20), Some(99999)),
        // Different pct values — should be kept
        make_record(4000, "s1", "/proj/a", 0.4, 800, 350, 8, 5, None, Some(15), Some(9999), Some(25), Some(99999)),
    ];

    let entries = aggregate_ratelimit(&records);
    assert_eq!(entries.len(), 2, "consecutive same values should be deduped");
    assert_eq!(entries[0].five_hour_pct, 10);
    assert_eq!(entries[1].five_hour_pct, 15);
}

#[test]
fn test_ratelimit_keeps_first_of_each_unique_pair() {
    // Pattern: (10,20), (10,21), (10,20) — (10,20) appears again after (10,21)
    let records = vec![
        make_record(1000, "s1", "/proj/a", 0.1, 500, 200, 5, 2, None, Some(10), Some(9999), Some(20), Some(99999)),
        make_record(2000, "s1", "/proj/a", 0.2, 600, 250, 6, 3, None, Some(10), Some(9999), Some(21), Some(99999)),
        make_record(3000, "s1", "/proj/a", 0.3, 700, 300, 7, 4, None, Some(10), Some(9999), Some(20), Some(99999)),
    ];

    let entries = aggregate_ratelimit(&records);
    // (10,20), (10,21), (10,20) — each is a change from previous, so all 3 kept
    assert_eq!(entries.len(), 3);
}

#[test]
fn test_ratelimit_empty_input() {
    let entries = aggregate_ratelimit(&[]);
    assert_eq!(entries.len(), 0);
}

// ─── aggregate_windows ────────────────────────────────────────────────────────

#[test]
fn test_window_aggregation_basic() {
    // Window resets_at = 1000 + 5*3600 = 19000 seconds mark
    let resets_at: i64 = 1_774_500_000; // some window boundary
    let records = vec![
        make_record(1_774_490_000, "s1", "/proj/a", 0.3, 1000, 400, 10, 5, None,
            Some(30), Some(resets_at), Some(50), Some(resets_at + 100000)),
        make_record(1_774_495_000, "s2", "/proj/b", 0.5, 2000, 800, 20, 10, None,
            Some(40), Some(resets_at), Some(60), Some(resets_at + 100000)),
    ];

    let sessions = aggregate_sessions(&records);
    let windows = aggregate_windows(&records, &sessions, WindowType::FiveHour, false);

    assert_eq!(windows.len(), 1);
    let w = &windows[0];
    assert_eq!(w.window_end.timestamp(), resets_at);
    assert_eq!(w.window_start.timestamp(), resets_at - 5 * 3600);
    assert_eq!(w.min_five_hour_pct, 30);
    assert_eq!(w.max_five_hour_pct, 40);
    assert_eq!(w.sessions, 2);
    // total_cost = max of s1 (0.3) + max of s2 (0.5) = 0.8
    assert!((w.total_cost - 0.8).abs() < 1e-9, "total_cost={}", w.total_cost);
    // est_budget = 0.8 * 100 / (40-30) = 8.0  (delta of 5h%)
    let est = w.est_budget.unwrap();
    assert!((est - 8.0).abs() < 1e-9, "est_budget={}", est);
}

#[test]
fn test_window_multiple_windows() {
    let resets_a: i64 = 1_774_500_000;
    let resets_b: i64 = 1_774_518_000; // different window

    let records = vec![
        make_record(1_774_490_000, "s1", "/proj/a", 0.2, 1000, 400, 10, 5, None,
            Some(20), Some(resets_a), Some(30), Some(resets_a + 100000)),
        make_record(1_774_510_000, "s2", "/proj/b", 0.4, 2000, 800, 20, 10, None,
            Some(40), Some(resets_b), Some(50), Some(resets_b + 100000)),
    ];

    let sessions = aggregate_sessions(&records);
    let windows = aggregate_windows(&records, &sessions, WindowType::FiveHour, false);

    assert_eq!(windows.len(), 2);
}

#[test]
fn test_window_no_ratelimit_records_excluded() {
    // Records without five_hour_resets_at should not appear in any window
    let records = vec![
        make_record(1_774_490_000, "s1", "/proj/a", 0.2, 1000, 400, 10, 5, None,
            None, None, None, None),
    ];

    let sessions = aggregate_sessions(&records);
    let windows = aggregate_windows(&records, &sessions, WindowType::FiveHour, false);
    assert_eq!(windows.len(), 0);
}

#[test]
fn test_window_zero_peak_pct_no_est_budget() {
    let resets_at: i64 = 1_774_500_000;
    let records = vec![
        make_record(1_774_490_000, "s1", "/proj/a", 0.3, 1000, 400, 10, 5, None,
            Some(0), Some(resets_at), Some(0), Some(resets_at + 100000)),
    ];

    let sessions = aggregate_sessions(&records);
    let windows = aggregate_windows(&records, &sessions, WindowType::FiveHour, false);
    assert_eq!(windows.len(), 1);
    assert_eq!(windows[0].max_five_hour_pct, 0);
    assert!(windows[0].est_budget.is_none(), "est_budget should be None when delta_pct=0");
}

// ─── aggregate_by_project ────────────────────────────────────────────────────

fn make_session(session_id: &str, project: &str, cost: f64, dur: u64, api_dur: u64,
    first_ts_secs: i64) -> SlSessionSummary {
    SlSessionSummary {
        session_id: session_id.to_string(),
        project: project.to_string(),
        model_name: "Claude Sonnet".to_string(),
        version: "1.0.0".to_string(),
        segments: 1,
        total_cost: cost,
        total_duration_ms: dur,
        total_api_duration_ms: api_dur,
        total_lines_added: 0,
        total_lines_removed: 0,
        max_context_pct: None,
        first_ts: Utc.timestamp_opt(first_ts_secs, 0).single().unwrap(),
        last_ts: Utc.timestamp_opt(first_ts_secs + 1000, 0).single().unwrap(),
        min_five_hour_pct: None,
        max_five_hour_pct: None,
        min_seven_day_pct: None,
        max_seven_day_pct: None,
    }
}

#[test]
fn test_project_aggregation_basic() {
    let sessions = vec![
        make_session("s1", "/proj/a", 0.1, 1000, 400, 1000),
        make_session("s2", "/proj/a", 0.3, 2000, 800, 2000),
        make_session("s3", "/proj/b", 0.5, 3000, 1200, 3000),
    ];

    let projects = aggregate_by_project(&sessions);
    assert_eq!(projects.len(), 2);

    let pa = projects.iter().find(|p| p.project == "/proj/a").unwrap();
    let pb = projects.iter().find(|p| p.project == "/proj/b").unwrap();

    assert!((pa.total_cost - 0.4).abs() < 1e-9, "proj/a total_cost={}", pa.total_cost);
    assert_eq!(pa.total_duration_ms, 3000);
    assert_eq!(pa.total_api_duration_ms, 1200);
    assert_eq!(pa.session_count, 2);

    assert!((pb.total_cost - 0.5).abs() < 1e-9);
    assert_eq!(pb.session_count, 1);
}

#[test]
fn test_project_aggregation_single_project() {
    let sessions = vec![
        make_session("s1", "/proj/only", 0.2, 1000, 400, 1000),
        make_session("s2", "/proj/only", 0.4, 2000, 800, 2000),
    ];

    let projects = aggregate_by_project(&sessions);
    assert_eq!(projects.len(), 1);
    assert!((projects[0].total_cost - 0.6).abs() < 1e-9);
    assert_eq!(projects[0].session_count, 2);
}

#[test]
fn test_project_aggregation_empty() {
    let projects = aggregate_by_project(&[]);
    assert_eq!(projects.len(), 0);
}

// ─── aggregate_by_day ────────────────────────────────────────────────────────

fn make_session_with_pct(
    session_id: &str, project: &str, cost: f64, first_ts_secs: i64,
    five_hour_pct: Option<u8>, seven_day_pct: Option<u8>,
) -> SlSessionSummary {
    SlSessionSummary {
        session_id: session_id.to_string(),
        project: project.to_string(),
        model_name: "Claude Sonnet".to_string(),
        version: "1.0.0".to_string(),
        segments: 1,
        total_cost: cost,
        total_duration_ms: 1000,
        total_api_duration_ms: 400,
        total_lines_added: 0,
        total_lines_removed: 0,
        max_context_pct: None,
        first_ts: Utc.timestamp_opt(first_ts_secs, 0).single().unwrap(),
        last_ts: Utc.timestamp_opt(first_ts_secs + 1000, 0).single().unwrap(),
        min_five_hour_pct: five_hour_pct,
        max_five_hour_pct: five_hour_pct,
        min_seven_day_pct: seven_day_pct,
        max_seven_day_pct: seven_day_pct,
    }
}

// 2026-03-26T00:00:00Z = 1774483200
// 2026-03-27T00:00:00Z = 1774569600
// 2026-03-28T00:00:00Z = 1774656000

#[test]
fn test_day_aggregation_utc() {
    let sessions = vec![
        make_session_with_pct("s1", "/proj/a", 0.1, 1774483200, Some(10), Some(20)), // 2026-03-26
        make_session_with_pct("s2", "/proj/a", 0.2, 1774490000, Some(15), Some(25)), // 2026-03-26
        make_session_with_pct("s3", "/proj/b", 0.3, 1774569600, Some(5), Some(30)),  // 2026-03-27
    ];

    let days = aggregate_by_day(&sessions, Some("UTC"));
    assert_eq!(days.len(), 2);

    let d26 = days.iter().find(|d| d.date == "2026-03-26").unwrap();
    let d27 = days.iter().find(|d| d.date == "2026-03-27").unwrap();

    assert!((d26.total_cost - 0.3).abs() < 1e-9, "2026-03-26 total_cost={}", d26.total_cost);
    assert_eq!(d26.session_count, 2);
    assert_eq!(d26.min_five_hour_pct, Some(10), "should be min of 10 and 15");
    assert_eq!(d26.max_five_hour_pct, Some(15), "should be max of 10 and 15");
    assert_eq!(d26.min_seven_day_pct, Some(20), "should be min of 20 and 25");
    assert_eq!(d26.max_seven_day_pct, Some(25), "should be max of 20 and 25");

    assert!((d27.total_cost - 0.3).abs() < 1e-9);
    assert_eq!(d27.session_count, 1);
}

#[test]
fn test_day_aggregation_sorted_by_date() {
    let sessions = vec![
        make_session_with_pct("s3", "/proj/a", 0.3, 1774656000, None, None), // 2026-03-28
        make_session_with_pct("s1", "/proj/a", 0.1, 1774483200, None, None), // 2026-03-26
        make_session_with_pct("s2", "/proj/a", 0.2, 1774569600, None, None), // 2026-03-27
    ];

    let days = aggregate_by_day(&sessions, Some("UTC"));
    assert_eq!(days.len(), 3);
    // BTreeMap ordering ensures sorted by date
    assert_eq!(days[0].date, "2026-03-26");
    assert_eq!(days[1].date, "2026-03-27");
    assert_eq!(days[2].date, "2026-03-28");
}

#[test]
fn test_day_aggregation_none_pct() {
    let sessions = vec![
        make_session_with_pct("s1", "/proj/a", 0.5, 1774483200, None, None),
    ];

    let days = aggregate_by_day(&sessions, Some("UTC"));
    assert_eq!(days.len(), 1);
    assert_eq!(days[0].max_five_hour_pct, None);
    assert_eq!(days[0].max_seven_day_pct, None);
}

#[test]
fn test_day_aggregation_fixed_offset() {
    // UTC+8: 1774483200 = 2026-03-26T00:00:00Z = 2026-03-26T08:00:00+08:00 → date 2026-03-26
    // 1774476000 = 2026-03-25T22:00:00Z = 2026-03-26T06:00:00+08:00 → date 2026-03-26 in +08:00
    let sessions = vec![
        make_session_with_pct("s1", "/proj/a", 0.1, 1774476000, None, None),
        make_session_with_pct("s2", "/proj/a", 0.2, 1774483200, None, None),
    ];

    let days_utc = aggregate_by_day(&sessions, Some("UTC"));
    let days_plus8 = aggregate_by_day(&sessions, Some("+08:00"));

    // In UTC: s1 is 2026-03-25, s2 is 2026-03-26 → 2 days
    assert_eq!(days_utc.len(), 2);
    // In +08:00: both are 2026-03-26 → 1 day
    assert_eq!(days_plus8.len(), 1, "both sessions should land on same day in UTC+8");
    assert_eq!(days_plus8[0].date, "2026-03-26");
    assert!((days_plus8[0].total_cost - 0.3).abs() < 1e-9);
}

#[test]
fn test_day_aggregation_empty() {
    let days = aggregate_by_day(&[], Some("UTC"));
    assert_eq!(days.len(), 0);
}
