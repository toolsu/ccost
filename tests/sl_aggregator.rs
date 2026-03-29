use ccost::sl::types::*;
use ccost::sl::{
    aggregate_by_day, aggregate_by_project, aggregate_ratelimit, aggregate_sessions,
    aggregate_windows, filter_windows_by_range, WindowType,
};
use chrono::{TimeZone, Utc};

// ─── Helper to build a minimal SlRecord ──────────────────────────────────────

#[allow(clippy::too_many_arguments)]
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
        five_hour_resets_at: five_hour_resets_at.and_then(|s| Utc.timestamp_opt(s, 0).single()),
        seven_day_pct,
        seven_day_resets_at: seven_day_resets_at.and_then(|s| Utc.timestamp_opt(s, 0).single()),
    }
}

// ─── aggregate_sessions ───────────────────────────────────────────────────────

#[test]
fn test_single_segment_session() {
    // Three records for one session with monotonically increasing cumulative values
    let records = vec![
        make_record(
            1000,
            "s1",
            "/proj/a",
            0.1,
            500,
            200,
            5,
            2,
            Some(10),
            None,
            None,
            None,
            None,
        ),
        make_record(
            2000,
            "s1",
            "/proj/a",
            0.3,
            1500,
            700,
            12,
            5,
            Some(20),
            None,
            None,
            None,
            None,
        ),
        make_record(
            3000,
            "s1",
            "/proj/a",
            0.5,
            2500,
            1100,
            20,
            9,
            Some(30),
            None,
            None,
            None,
            None,
        ),
    ];

    let summaries = aggregate_sessions(&records);
    assert_eq!(summaries.len(), 1);

    let s = &summaries[0];
    assert_eq!(s.session_id, "s1");
    assert_eq!(s.segments, 1);
    // Delta: max minus first record's baseline (0.5-0.1, 2500-500, 1100-200, 20-5, 9-2)
    assert!(
        (s.total_cost - 0.4).abs() < 1e-9,
        "total_cost={}",
        s.total_cost
    );
    assert_eq!(s.total_duration_ms, 2000);
    assert_eq!(s.total_api_duration_ms, 900);
    assert_eq!(s.total_lines_added, 15);
    assert_eq!(s.total_lines_removed, 7);
    assert_eq!(s.max_context_pct, Some(30));
    assert_eq!(s.first_ts.timestamp(), 1000);
    assert_eq!(s.last_ts.timestamp(), 3000);
}

#[test]
fn test_multi_segment_session_cost_reset() {
    // Segment 1: records at ts 1000, 2000 with cost going 0.1 -> 0.5
    // Segment 2: cost drops to 0.1 (reset), then rises to 0.3
    let records = vec![
        make_record(
            1000, "s1", "/proj/a", 0.1, 500, 200, 5, 2, None, None, None, None, None,
        ),
        make_record(
            2000, "s1", "/proj/a", 0.5, 1500, 700, 15, 5, None, None, None, None, None,
        ),
        // Reset: cost dropped significantly
        make_record(
            3000, "s1", "/proj/a", 0.1, 500, 200, 5, 2, None, None, None, None, None,
        ),
        make_record(
            4000, "s1", "/proj/a", 0.3, 1000, 400, 10, 4, None, None, None, None, None,
        ),
    ];

    let summaries = aggregate_sessions(&records);
    assert_eq!(summaries.len(), 1);

    let s = &summaries[0];
    assert_eq!(s.segments, 2, "should detect 2 segments");
    // Seg1 max: 0.5,1500; Seg2 max: 0.3,1000; sum=0.8,2500; minus baseline(0.1,500) → 0.7,2000
    assert!(
        (s.total_cost - 0.7).abs() < 1e-9,
        "total_cost={}",
        s.total_cost
    );
    assert_eq!(s.total_duration_ms, 2000);
}

#[test]
fn test_multi_segment_session_duration_reset() {
    // Duration drops triggers a new segment
    let records = vec![
        make_record(
            1000, "s1", "/proj/a", 0.2, 2000, 800, 10, 5, None, None, None, None, None,
        ),
        make_record(
            2000, "s1", "/proj/a", 0.5, 5000, 2000, 20, 10, None, None, None, None, None,
        ),
        // Duration drops from 5000 to 200 (> 100 difference) and cost also drops
        make_record(
            3000, "s1", "/proj/a", 0.1, 200, 100, 5, 2, None, None, None, None, None,
        ),
        make_record(
            4000, "s1", "/proj/a", 0.4, 3000, 1500, 18, 8, None, None, None, None, None,
        ),
    ];

    let summaries = aggregate_sessions(&records);
    assert_eq!(summaries.len(), 1);

    let s = &summaries[0];
    assert_eq!(s.segments, 2, "duration drop should trigger new segment");
    // Seg1 max: 0.5,5000; Seg2 max: 0.4,3000; sum=0.9,8000; minus baseline(0.2,2000) → 0.7,6000
    assert!((s.total_cost - 0.7).abs() < 1e-9);
    assert_eq!(s.total_duration_ms, 6000);
}

#[test]
fn test_zero_zero_not_treated_as_reset() {
    // Two consecutive zero records should NOT be treated as a reset
    let records = vec![
        make_record(
            1000, "s1", "/proj/a", 0.0, 0, 0, 0, 0, None, None, None, None, None,
        ),
        make_record(
            2000, "s1", "/proj/a", 0.0, 0, 0, 0, 0, None, None, None, None, None,
        ),
        make_record(
            3000, "s1", "/proj/a", 0.2, 500, 200, 5, 2, None, None, None, None, None,
        ),
    ];

    let summaries = aggregate_sessions(&records);
    assert_eq!(summaries.len(), 1);
    assert_eq!(summaries[0].segments, 1, "0->0 should not be a reset");
}

#[test]
fn test_zero_nonzero_then_drop_is_reset() {
    // 0 -> 0.5 -> 0.1 (drop) should detect reset at the third record
    let records = vec![
        make_record(
            1000, "s1", "/proj/a", 0.0, 0, 0, 0, 0, None, None, None, None, None,
        ),
        make_record(
            2000, "s1", "/proj/a", 0.5, 1000, 400, 10, 5, None, None, None, None, None,
        ),
        make_record(
            3000, "s1", "/proj/a", 0.1, 300, 100, 3, 1, None, None, None, None, None,
        ),
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
        make_record(
            1000,
            "s1",
            "/proj/a",
            0.1,
            500,
            200,
            5,
            2,
            Some(5),
            None,
            None,
            None,
            None,
        ),
        make_record(
            2000,
            "s1",
            "/proj/a",
            0.3,
            1500,
            600,
            12,
            5,
            Some(15),
            None,
            None,
            None,
            None,
        ),
        make_record(
            1500,
            "s2",
            "/proj/b",
            0.2,
            800,
            300,
            8,
            3,
            Some(8),
            None,
            None,
            None,
            None,
        ),
        make_record(
            2500,
            "s2",
            "/proj/b",
            0.6,
            2000,
            900,
            25,
            10,
            Some(20),
            None,
            None,
            None,
            None,
        ),
    ];

    let summaries = aggregate_sessions(&records);
    assert_eq!(summaries.len(), 2);

    let s1 = summaries.iter().find(|s| s.session_id == "s1").unwrap();
    let s2 = summaries.iter().find(|s| s.session_id == "s2").unwrap();

    // s1: max=0.3 - baseline=0.1 = 0.2; s2: max=0.6 - baseline=0.2 = 0.4
    assert!((s1.total_cost - 0.2).abs() < 1e-9);
    assert!((s2.total_cost - 0.4).abs() < 1e-9);
    assert_eq!(s1.max_context_pct, Some(15));
    assert_eq!(s2.max_context_pct, Some(20));
}

#[test]
fn test_continued_session_baseline_subtraction() {
    // Simulates `claude --continue`: session B inherits cumulative values from session A.
    // Session A: $0 → $10 (fresh start)
    // Session B: $10 → $10.50 (continued, inherits $10 baseline)
    let records = vec![
        // Session A: fresh start
        make_record(
            1000, "sA", "/proj/a", 0.0, 0, 0, 0, 0, None, None, None, None, None,
        ),
        make_record(
            2000, "sA", "/proj/a", 5.0, 3000, 1000, 100, 20, None, None, None, None, None,
        ),
        make_record(
            3000, "sA", "/proj/a", 10.0, 6000, 2000, 200, 40, None, None, None, None, None,
        ),
        // Session B: continued from A (inherits $10, 6000ms, etc.)
        make_record(
            4000, "sB", "/proj/a", 10.0, 6000, 2000, 200, 40, None, None, None, None, None,
        ),
        make_record(
            5000, "sB", "/proj/a", 10.3, 6500, 2200, 210, 42, None, None, None, None, None,
        ),
        make_record(
            6000, "sB", "/proj/a", 10.5, 7000, 2400, 220, 45, None, None, None, None, None,
        ),
    ];

    let summaries = aggregate_sessions(&records);
    assert_eq!(summaries.len(), 2);

    let sa = summaries.iter().find(|s| s.session_id == "sA").unwrap();
    let sb = summaries.iter().find(|s| s.session_id == "sB").unwrap();

    // Session A: baseline=0, delta = $10
    assert!(
        (sa.total_cost - 10.0).abs() < 1e-9,
        "sA cost={}",
        sa.total_cost
    );
    assert_eq!(sa.total_duration_ms, 6000);
    assert_eq!(sa.total_lines_added, 200);

    // Session B: baseline=$10, delta = $0.50 (NOT $10.50!)
    assert!(
        (sb.total_cost - 0.5).abs() < 1e-9,
        "sB cost={} (should be 0.5, not 10.5)",
        sb.total_cost
    );
    assert_eq!(sb.total_duration_ms, 1000); // 7000 - 6000
    assert_eq!(sb.total_api_duration_ms, 400); // 2400 - 2000
    assert_eq!(sb.total_lines_added, 20); // 220 - 200
    assert_eq!(sb.total_lines_removed, 5); // 45 - 40
}

#[test]
fn test_continued_session_with_reset() {
    // Continued session that also has an internal reset
    let records = vec![
        // Session: continued with baseline=$10, then resets mid-session
        make_record(
            1000, "s1", "/proj/a", 10.0, 6000, 2000, 200, 40, None, None, None, None, None,
        ),
        make_record(
            2000, "s1", "/proj/a", 10.5, 6500, 2200, 210, 42, None, None, None, None, None,
        ),
        // Reset: counters drop
        make_record(
            3000, "s1", "/proj/a", 0.2, 300, 100, 5, 2, None, None, None, None, None,
        ),
        make_record(
            4000, "s1", "/proj/a", 0.8, 1000, 400, 15, 5, None, None, None, None, None,
        ),
    ];

    let summaries = aggregate_sessions(&records);
    assert_eq!(summaries.len(), 1);

    let s = &summaries[0];
    assert_eq!(s.segments, 2);
    // Seg1: max=10.5, Seg2: max=0.8; raw_total=11.3; minus baseline=10.0 → 1.3
    // = 0.5 (work before reset) + 0.8 (work after reset)
    assert!((s.total_cost - 1.3).abs() < 1e-9, "cost={}", s.total_cost);
    assert_eq!(s.total_duration_ms, 1500); // (6500+1000) - 6000
}

#[test]
fn test_last_ts_fields() {
    let records = vec![
        make_record(
            1000,
            "s1",
            "/proj/a",
            0.1,
            500,
            200,
            5,
            2,
            None,
            Some(10),
            Some(9999),
            Some(20),
            Some(99999),
        ),
        make_record(
            2000,
            "s1",
            "/proj/a",
            0.3,
            1500,
            600,
            12,
            5,
            None,
            Some(30),
            Some(9999),
            Some(50),
            Some(99999),
        ),
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
        make_record(
            1000,
            "s1",
            "/proj/a",
            0.1,
            500,
            200,
            5,
            2,
            None,
            Some(10),
            Some(9999),
            None,
            None,
        ),
        // All present
        make_record(
            2000,
            "s1",
            "/proj/a",
            0.2,
            600,
            250,
            6,
            3,
            None,
            Some(20),
            Some(9999),
            Some(30),
            Some(99999),
        ),
        // Missing five_hour_resets_at
        make_record(
            3000,
            "s1",
            "/proj/a",
            0.3,
            700,
            300,
            7,
            4,
            None,
            Some(25),
            None,
            Some(35),
            Some(99999),
        ),
    ];

    let entries = aggregate_ratelimit(&records);
    assert_eq!(entries.len(), 1, "only fully-populated records should pass");
    assert_eq!(entries[0].five_hour_pct, 20);
    assert_eq!(entries[0].seven_day_pct, 30);
}

#[test]
fn test_ratelimit_dedup_consecutive_same() {
    let records = vec![
        make_record(
            1000,
            "s1",
            "/proj/a",
            0.1,
            500,
            200,
            5,
            2,
            None,
            Some(10),
            Some(9999),
            Some(20),
            Some(99999),
        ),
        make_record(
            2000,
            "s1",
            "/proj/a",
            0.2,
            600,
            250,
            6,
            3,
            None,
            Some(10),
            Some(9999),
            Some(20),
            Some(99999),
        ),
        make_record(
            3000,
            "s1",
            "/proj/a",
            0.3,
            700,
            300,
            7,
            4,
            None,
            Some(10),
            Some(9999),
            Some(20),
            Some(99999),
        ),
        // Different pct values — should be kept
        make_record(
            4000,
            "s1",
            "/proj/a",
            0.4,
            800,
            350,
            8,
            5,
            None,
            Some(15),
            Some(9999),
            Some(25),
            Some(99999),
        ),
    ];

    let entries = aggregate_ratelimit(&records);
    assert_eq!(
        entries.len(),
        2,
        "consecutive same values should be deduped"
    );
    assert_eq!(entries[0].five_hour_pct, 10);
    assert_eq!(entries[1].five_hour_pct, 15);
}

#[test]
fn test_ratelimit_keeps_first_of_each_unique_pair() {
    // Pattern: (10,20), (10,21), (10,20) — (10,20) appears again after (10,21)
    let records = vec![
        make_record(
            1000,
            "s1",
            "/proj/a",
            0.1,
            500,
            200,
            5,
            2,
            None,
            Some(10),
            Some(9999),
            Some(20),
            Some(99999),
        ),
        make_record(
            2000,
            "s1",
            "/proj/a",
            0.2,
            600,
            250,
            6,
            3,
            None,
            Some(10),
            Some(9999),
            Some(21),
            Some(99999),
        ),
        make_record(
            3000,
            "s1",
            "/proj/a",
            0.3,
            700,
            300,
            7,
            4,
            None,
            Some(10),
            Some(9999),
            Some(20),
            Some(99999),
        ),
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

#[test]
fn test_action_cost_not_lost_on_constant_pct() {
    // Two sessions where pct never changes — cost should not be lost at EOF.
    // Session s1: cost 0.0 -> 1.0 -> 2.0 at constant (10%, 5%)
    // Session s2: cost 0.0 -> 0.5 -> 1.0 at constant (10%, 5%)
    // Only the first record of each session is emitted (pair dedup).
    // Without the EOF flush fix, total action cost = $0.00 (all deltas lost).
    // With fix, total action cost = $3.00 ($2.00 from s1 + $1.00 from s2).
    let resets_5h: i64 = 1774497600;
    let resets_7d: i64 = 1774605600;

    let records = vec![
        // Session s1: 3 records, all at (10%, 5%)
        make_record(
            1000,
            "s1",
            "/proj/a",
            0.0,
            500,
            200,
            5,
            2,
            None,
            Some(10),
            Some(resets_5h),
            Some(5),
            Some(resets_7d),
        ),
        make_record(
            2000,
            "s1",
            "/proj/a",
            1.0,
            1000,
            400,
            10,
            4,
            None,
            Some(10),
            Some(resets_5h),
            Some(5),
            Some(resets_7d),
        ),
        make_record(
            3000,
            "s1",
            "/proj/a",
            2.0,
            1500,
            600,
            15,
            6,
            None,
            Some(10),
            Some(resets_5h),
            Some(5),
            Some(resets_7d),
        ),
        // Session s2: 3 records, all at (10%, 5%)
        make_record(
            4000,
            "s2",
            "/proj/b",
            0.0,
            500,
            200,
            5,
            2,
            None,
            Some(10),
            Some(resets_5h),
            Some(5),
            Some(resets_7d),
        ),
        make_record(
            5000,
            "s2",
            "/proj/b",
            0.5,
            1000,
            400,
            10,
            4,
            None,
            Some(10),
            Some(resets_5h),
            Some(5),
            Some(resets_7d),
        ),
        make_record(
            6000,
            "s2",
            "/proj/b",
            1.0,
            1500,
            600,
            15,
            6,
            None,
            Some(10),
            Some(resets_5h),
            Some(5),
            Some(resets_7d),
        ),
    ];

    let entries = aggregate_ratelimit(&records);
    // Only two entries emitted (first of each session, since pct never changes)
    // But each should capture remaining cost via EOF flush.
    let total_cost: f64 = entries.iter().map(|e| e.cost_delta).sum();
    assert!(
        (total_cost - 3.0).abs() < 1e-9,
        "total action cost should be $3.00 (s1=$2.00 + s2=$1.00), got ${:.2}",
        total_cost
    );

    // Verify per-session: s1 should have $2.00, s2 should have $1.00
    let s1_cost: f64 = entries
        .iter()
        .filter(|e| e.session_id == "s1")
        .map(|e| e.cost_delta)
        .sum();
    let s2_cost: f64 = entries
        .iter()
        .filter(|e| e.session_id == "s2")
        .map(|e| e.cost_delta)
        .sum();
    assert!(
        (s1_cost - 2.0).abs() < 1e-9,
        "s1 cost should be $2.00, got ${:.2}",
        s1_cost
    );
    assert!(
        (s2_cost - 1.0).abs() < 1e-9,
        "s2 cost should be $1.00, got ${:.2}",
        s2_cost
    );
}

// ─── aggregate_windows ────────────────────────────────────────────────────────

#[test]
fn test_window_aggregation_basic() {
    // Window resets_at = 1000 + 5*3600 = 19000 seconds mark
    let resets_at: i64 = 1_774_500_000; // some window boundary
    let window_start = resets_at - 5 * 3600; // 1_774_482_000
    let records = vec![
        // Session baselines (before window start, cost=0 = fresh sessions)
        make_record(
            window_start - 100,
            "s1",
            "/proj/a",
            0.0,
            0,
            0,
            0,
            0,
            None,
            None,
            None,
            None,
            None,
        ),
        make_record(
            window_start - 100,
            "s2",
            "/proj/b",
            0.0,
            0,
            0,
            0,
            0,
            None,
            None,
            None,
            None,
            None,
        ),
        make_record(
            1_774_490_000,
            "s1",
            "/proj/a",
            0.3,
            1000,
            400,
            10,
            5,
            None,
            Some(30),
            Some(resets_at),
            Some(50),
            Some(resets_at + 100000),
        ),
        make_record(
            1_774_495_000,
            "s2",
            "/proj/b",
            0.5,
            2000,
            800,
            20,
            10,
            None,
            Some(40),
            Some(resets_at),
            Some(60),
            Some(resets_at + 100000),
        ),
    ];

    let sessions = aggregate_sessions(&records);
    let windows = aggregate_windows(&records, &sessions, WindowType::FiveHour, false);

    assert_eq!(windows.len(), 1);
    let w = &windows[0];
    assert_eq!(w.window_end.timestamp(), resets_at);
    assert_eq!(w.window_start.timestamp(), window_start);
    assert_eq!(w.min_five_hour_pct, 30);
    assert_eq!(w.max_five_hour_pct, 40);
    assert_eq!(w.sessions, 2);
    // total_cost = delta of s1 (0.3-0) + delta of s2 (0.5-0) = 0.8
    assert!(
        (w.total_cost - 0.8).abs() < 1e-9,
        "total_cost={}",
        w.total_cost
    );
    // est_5h_budget = 0.8 * 100 / (40-30) = 8.0  (delta of 5h%)
    let est = w.est_5h_budget.unwrap();
    assert!((est - 8.0).abs() < 1e-9, "est_5h_budget={}", est);
}

#[test]
fn test_window_continued_session() {
    // A continued session within a window should only count its delta, not inherited baseline.
    let resets_at: i64 = 1_774_500_000;
    let window_start = resets_at - 5 * 3600;
    let records = vec![
        // Continued session: first record has inherited $10 baseline, appears within the window
        make_record(
            window_start + 1000,
            "s1",
            "/proj/a",
            10.0,
            6000,
            2000,
            200,
            40,
            None,
            Some(30),
            Some(resets_at),
            Some(50),
            Some(resets_at + 100000),
        ),
        make_record(
            window_start + 2000,
            "s1",
            "/proj/a",
            10.5,
            6500,
            2200,
            210,
            42,
            None,
            Some(35),
            Some(resets_at),
            Some(55),
            Some(resets_at + 100000),
        ),
    ];

    let sessions = aggregate_sessions(&records);
    let windows = aggregate_windows(&records, &sessions, WindowType::FiveHour, false);

    assert_eq!(windows.len(), 1);
    let w = &windows[0];
    // Should be $0.50 delta, NOT $10.50 cumulative
    assert!(
        (w.total_cost - 0.5).abs() < 1e-9,
        "window cost={} (should be 0.5, not 10.5)",
        w.total_cost
    );
}

#[test]
fn test_window_multiple_windows() {
    let resets_a: i64 = 1_774_500_000;
    let resets_b: i64 = 1_774_518_000; // different window

    let records = vec![
        make_record(
            1_774_490_000,
            "s1",
            "/proj/a",
            0.2,
            1000,
            400,
            10,
            5,
            None,
            Some(20),
            Some(resets_a),
            Some(30),
            Some(resets_a + 100000),
        ),
        make_record(
            1_774_510_000,
            "s2",
            "/proj/b",
            0.4,
            2000,
            800,
            20,
            10,
            None,
            Some(40),
            Some(resets_b),
            Some(50),
            Some(resets_b + 100000),
        ),
    ];

    let sessions = aggregate_sessions(&records);
    let windows = aggregate_windows(&records, &sessions, WindowType::FiveHour, false);

    assert_eq!(windows.len(), 2);
}

#[test]
fn test_window_no_ratelimit_records_excluded() {
    // Records without five_hour_resets_at should not appear in any window
    let records = vec![make_record(
        1_774_490_000,
        "s1",
        "/proj/a",
        0.2,
        1000,
        400,
        10,
        5,
        None,
        None,
        None,
        None,
        None,
    )];

    let sessions = aggregate_sessions(&records);
    let windows = aggregate_windows(&records, &sessions, WindowType::FiveHour, false);
    assert_eq!(windows.len(), 0);
}

#[test]
fn test_window_zero_peak_pct_no_est_budget() {
    let resets_at: i64 = 1_774_500_000;
    let records = vec![make_record(
        1_774_490_000,
        "s1",
        "/proj/a",
        0.3,
        1000,
        400,
        10,
        5,
        None,
        Some(0),
        Some(resets_at),
        Some(0),
        Some(resets_at + 100000),
    )];

    let sessions = aggregate_sessions(&records);
    let windows = aggregate_windows(&records, &sessions, WindowType::FiveHour, false);
    assert_eq!(windows.len(), 1);
    assert_eq!(windows[0].max_five_hour_pct, 0);
    assert!(
        windows[0].est_5h_budget.is_none(),
        "est_5h_budget should be None when delta_pct=0"
    );
}

// ─── aggregate_by_project ────────────────────────────────────────────────────

fn make_session(
    session_id: &str,
    project: &str,
    cost: f64,
    dur: u64,
    api_dur: u64,
    first_ts_secs: i64,
) -> SlSessionSummary {
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

    assert!(
        (pa.total_cost - 0.4).abs() < 1e-9,
        "proj/a total_cost={}",
        pa.total_cost
    );
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
    session_id: &str,
    project: &str,
    cost: f64,
    first_ts_secs: i64,
    five_hour_pct: Option<u8>,
    seven_day_pct: Option<u8>,
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

    assert!(
        (d26.total_cost - 0.3).abs() < 1e-9,
        "2026-03-26 total_cost={}",
        d26.total_cost
    );
    assert_eq!(d26.session_count, 2);
    assert_eq!(
        d26.min_five_hour_pct,
        Some(10),
        "should be min of 10 and 15"
    );
    assert_eq!(
        d26.max_five_hour_pct,
        Some(15),
        "should be max of 10 and 15"
    );
    assert_eq!(
        d26.min_seven_day_pct,
        Some(20),
        "should be min of 20 and 25"
    );
    assert_eq!(
        d26.max_seven_day_pct,
        Some(25),
        "should be max of 20 and 25"
    );

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
    let sessions = vec![make_session_with_pct(
        "s1", "/proj/a", 0.5, 1774483200, None, None,
    )];

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
    assert_eq!(
        days_plus8.len(),
        1,
        "both sessions should land on same day in UTC+8"
    );
    assert_eq!(days_plus8[0].date, "2026-03-26");
    assert!((days_plus8[0].total_cost - 0.3).abs() < 1e-9);
}

#[test]
fn test_day_aggregation_empty() {
    let days = aggregate_by_day(&[], Some("UTC"));
    assert_eq!(days.len(), 0);
}

// ─── filter_windows_by_range ─────────────────────────────────────────────────

/// Helper: build records spanning multiple 5h windows and return the aggregated windows.
fn make_multi_window_records() -> (Vec<SlRecord>, Vec<SlWindowSummary>) {
    // 2026-03-27T00:00:00Z = 1774569600
    // Window A: resets_at 2026-03-27T13:00:00Z, start 08:00 UTC
    // Window B: resets_at 2026-03-27T18:00:00Z, start 13:00 UTC
    // Window C: resets_at 2026-03-27T23:00:00Z, start 18:00 UTC
    let resets_a: i64 = 1774569600 + 13 * 3600; // 2026-03-27T13:00:00Z
    let resets_b: i64 = 1774569600 + 18 * 3600; // 2026-03-27T18:00:00Z
    let resets_c: i64 = 1774569600 + 23 * 3600; // 2026-03-27T23:00:00Z

    let records = vec![
        // Window A record (ts within 08:00–13:00)
        make_record(
            resets_a - 3600,
            "s1",
            "/proj/a",
            0.1,
            500,
            200,
            5,
            2,
            None,
            Some(10),
            Some(resets_a),
            Some(20),
            Some(resets_a + 604800),
        ),
        // Window B record (ts within 13:00–18:00)
        make_record(
            resets_b - 3600,
            "s2",
            "/proj/a",
            0.2,
            600,
            250,
            6,
            3,
            None,
            Some(20),
            Some(resets_b),
            Some(30),
            Some(resets_b + 604800),
        ),
        // Window C record (ts within 18:00–23:00)
        make_record(
            resets_c - 3600,
            "s3",
            "/proj/a",
            0.3,
            700,
            300,
            7,
            4,
            None,
            Some(30),
            Some(resets_c),
            Some(40),
            Some(resets_c + 604800),
        ),
    ];

    let sessions = aggregate_sessions(&records);
    let windows = aggregate_windows(&records, &sessions, WindowType::FiveHour, false);
    (records, windows)
}

#[test]
fn test_filter_windows_no_filters() {
    let (_, windows) = make_multi_window_records();
    assert_eq!(windows.len(), 3);

    // No filters — all windows returned
    let filtered = filter_windows_by_range(windows, &None, &None, Some("UTC"));
    assert_eq!(filtered.len(), 3);
}

#[test]
fn test_filter_windows_from_excludes_earlier() {
    let (_, windows) = make_multi_window_records();
    // --from 2026-03-27T18:00 should exclude windows ending at or before 18:00
    // Window A: 08:00–13:00 → end 13:00 <= 18:00 → excluded
    // Window B: 13:00–18:00 → end 18:00 <= 18:00 → excluded
    // Window C: 18:00–23:00 → end 23:00 > 18:00 → kept
    let from = Some("2026-03-27T18:00".to_string());
    let filtered = filter_windows_by_range(windows, &from, &None, Some("UTC"));
    assert_eq!(filtered.len(), 1);
    assert_eq!(filtered[0].window_start.timestamp(), 1774569600 + 18 * 3600); // 18:00 UTC
}

#[test]
fn test_filter_windows_to_excludes_later() {
    let (_, windows) = make_multi_window_records();
    // --to 2026-03-27T13:00 should exclude windows starting at or after 13:00
    // Window A: 08:00–13:00 → start 08:00 < 13:00 → kept
    // Window B: 13:00–18:00 → start 13:00 >= 13:00 → excluded
    // Window C: 18:00–23:00 → start 18:00 >= 13:00 → excluded
    let to = Some("2026-03-27T13:00".to_string());
    let filtered = filter_windows_by_range(windows, &None, &to, Some("UTC"));
    assert_eq!(filtered.len(), 1);
    assert_eq!(filtered[0].window_end.timestamp(), 1774569600 + 13 * 3600); // 13:00 UTC
}

#[test]
fn test_filter_windows_from_and_to() {
    let (_, windows) = make_multi_window_records();
    // --from 2026-03-27T12:00 --to 2026-03-27T19:00
    // Window A: 08:00–13:00 → end 13:00 > 12:00 ✓, start 08:00 < 19:00 ✓ → kept
    // Window B: 13:00–18:00 → end 18:00 > 12:00 ✓, start 13:00 < 19:00 ✓ → kept
    // Window C: 18:00–23:00 → end 23:00 > 12:00 ✓, start 18:00 < 19:00 ✓ → kept
    let from = Some("2026-03-27T12:00".to_string());
    let to = Some("2026-03-27T19:00".to_string());
    let filtered = filter_windows_by_range(windows, &from, &to, Some("UTC"));
    assert_eq!(filtered.len(), 3);
}

#[test]
fn test_filter_windows_narrow_range_keeps_overlapping() {
    let (_, windows) = make_multi_window_records();
    // --from 2026-03-27T14:00 --to 2026-03-27T17:00
    // Only Window B (13:00–18:00) overlaps: end 18:00 > 14:00 ✓, start 13:00 < 17:00 ✓
    // Window A: 08:00–13:00 → end 13:00 <= 14:00 → excluded
    // Window C: 18:00–23:00 → start 18:00 >= 17:00 → excluded
    let from = Some("2026-03-27T14:00".to_string());
    let to = Some("2026-03-27T17:00".to_string());
    let filtered = filter_windows_by_range(windows, &from, &to, Some("UTC"));
    assert_eq!(filtered.len(), 1);
    assert_eq!(filtered[0].window_start.timestamp(), 1774569600 + 13 * 3600); // 13:00 UTC
}

#[test]
fn test_filter_windows_timezone_aware() {
    let (_, windows) = make_multi_window_records();
    // In UTC+8, window times shift forward by 8 hours:
    // Window A: 16:00–21:00 (UTC+8)
    // Window B: 21:00–02:00+1 (UTC+8)
    // Window C: 02:00–07:00+1 (UTC+8)
    //
    // --from 2026-03-27T22:00 in UTC+8 (= 14:00 UTC)
    // Window A: end 21:00 <= 22:00 → excluded
    // Window B: end 02:00+1 > 22:00 ✓ → kept
    // Window C: end 07:00+1 > 22:00 ✓ → kept
    let from = Some("2026-03-27T22:00".to_string());
    let filtered = filter_windows_by_range(windows, &from, &None, Some("+08:00"));
    assert_eq!(filtered.len(), 2);
}

#[test]
fn test_filter_windows_empty_input() {
    let filtered = filter_windows_by_range(
        vec![],
        &Some("2026-03-27T18:00".to_string()),
        &None,
        Some("UTC"),
    );
    assert_eq!(filtered.len(), 0);
}
