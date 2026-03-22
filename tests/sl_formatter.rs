use ccost::sl::types::*;
use ccost::sl::formatter::*;
use ccost::types::PriceMode;
use chrono::{TimeZone, Utc};

// ─── Helpers ──────────────────────────────────────────────────────────────────

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
        cost_delta: 0.0,
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

fn make_json_meta(view: &str) -> SlJsonMeta {
    SlJsonMeta {
        source: "test".to_string(),
        file: "test.jsonl".to_string(),
        view: view.to_string(),
        from: None,
        to: None,
        tz: Some("UTC".to_string()),
        generated_at: "2026-03-26T00:00:00Z".to_string(),
    }
}

// ─── Duration formatting ──────────────────────────────────────────────────────

#[test]
fn test_fmt_duration_zero() {
    assert_eq!(fmt_duration(0), "0s");
}

#[test]
fn test_fmt_duration_seconds_only() {
    assert_eq!(fmt_duration(5_000), "5s");
    assert_eq!(fmt_duration(59_000), "59s");
}

#[test]
fn test_fmt_duration_minutes_and_seconds() {
    assert_eq!(fmt_duration(60_000), "1m 0s");
    assert_eq!(fmt_duration(90_000), "1m 30s");
    assert_eq!(fmt_duration(3_599_000), "59m 59s");
}

#[test]
fn test_fmt_duration_hours_and_minutes() {
    assert_eq!(fmt_duration(3_600_000), "1h 0m");
    assert_eq!(fmt_duration(3_660_000), "1h 1m");
    assert_eq!(fmt_duration(7_200_000), "2h 0m");
    assert_eq!(fmt_duration(7_320_000), "2h 2m");
}

#[test]
fn test_fmt_duration_large() {
    // 25 hours 30 minutes = 91800 seconds
    assert_eq!(fmt_duration(91_800_000), "25h 30m");
}

// ─── Project shortening ───────────────────────────────────────────────────────

#[test]
fn test_shorten_project_deep_path() {
    assert_eq!(shorten_project("/home/user/projects/foo/bar"), ".../foo/bar");
}

#[test]
fn test_shorten_project_three_components() {
    assert_eq!(shorten_project("/a/b/c"), ".../b/c");
}

#[test]
fn test_shorten_project_two_components() {
    // Exactly 2 components — don't shorten
    assert_eq!(shorten_project("/foo/bar"), "/foo/bar");
}

#[test]
fn test_shorten_project_one_component() {
    assert_eq!(shorten_project("/foo"), "/foo");
}

#[test]
fn test_shorten_project_no_slash() {
    assert_eq!(shorten_project("foo"), "foo");
}

// ─── Rate-limit table ─────────────────────────────────────────────────────────

#[test]
fn test_ratelimit_table_headers_full() {
    let entries = vec![make_ratelimit_entry(
        1_774_483_200, "session-abc123", 30, 1_774_500_000, 50, 1_775_000_000,
    )];
    let opts = SlFormatOptions {
        tz: Some("UTC".to_string()),
        price_mode: PriceMode::Decimal,
        compact: false,
        color: false,
    };
    let result = format_sl_ratelimit_table(&entries, &opts);

    // Check all expected column headers
    assert!(result.contains("Time"), "should contain Time");
    assert!(result.contains("5h%"), "should contain 5h%");
    assert!(result.contains("1w%"), "should contain 1w%");
    assert!(result.contains("5h Resets"), "should contain 5h Resets");
    assert!(result.contains("Session"), "should contain Session");
}

#[test]
fn test_ratelimit_table_compact_no_session_column() {
    let entries = vec![make_ratelimit_entry(
        1_774_483_200, "session-abc123", 30, 1_774_500_000, 50, 1_775_000_000,
    )];
    let opts = SlFormatOptions {
        tz: Some("UTC".to_string()),
        price_mode: PriceMode::Decimal,
        compact: true,
        color: false,
    };
    let result = format_sl_ratelimit_table(&entries, &opts);

    assert!(!result.contains("Session"), "compact should hide Session column");
    assert!(result.contains("5h%"), "should still contain 5h%");
    assert!(result.contains("1w%"), "should still contain 1w%");
}

#[test]
fn test_ratelimit_table_percentage_values() {
    let entries = vec![make_ratelimit_entry(
        1_774_483_200, "session-abc123", 45, 1_774_500_000, 72, 1_775_000_000,
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
}

#[test]
fn test_ratelimit_table_session_truncated_to_8() {
    // session_id = "session-abc123" → first 8 chars = "session-"
    let entries = vec![make_ratelimit_entry(
        1_774_483_200, "session-abc123", 30, 1_774_500_000, 50, 1_775_000_000,
    )];
    let opts = SlFormatOptions {
        tz: Some("UTC".to_string()),
        price_mode: PriceMode::Decimal,
        compact: false,
        color: false,
    };
    let result = format_sl_ratelimit_table(&entries, &opts);
    assert!(result.contains("session-"), "should contain first 8 chars");
    // Should NOT contain the full session id beyond 8 chars (we only truncate in the table)
    // "abc123" would appear only if the full id were shown
    // Since "session-abc123" truncated is "session-", "abc123" should not appear
    assert!(!result.contains("abc123"), "full session id beyond 8 chars should not appear");
}

#[test]
fn test_ratelimit_table_empty() {
    let opts = SlFormatOptions {
        tz: Some("UTC".to_string()),
        price_mode: PriceMode::Decimal,
        compact: false,
        color: false,
    };
    let result = format_sl_ratelimit_table(&[], &opts);
    // Still has headers in a box table
    assert!(result.contains("Time"), "empty table should still have headers");
}

#[test]
fn test_ratelimit_table_time_format() {
    // 2026-03-26T00:00:00Z should format as "03-26 00:00" in short format
    let entries = vec![make_ratelimit_entry(
        1_774_483_200, "s1", 30, 1_774_500_000, 50, 1_775_000_000,
    )];
    let opts = SlFormatOptions {
        tz: Some("UTC".to_string()),
        price_mode: PriceMode::Decimal,
        compact: false,
        color: false,
    };
    let result = format_sl_ratelimit_table(&entries, &opts);
    assert!(result.contains("03-26 00:00"), "time should be formatted as MM-DD HH:MM");
}

#[test]
fn test_ratelimit_table_is_box_drawing() {
    let entries = vec![make_ratelimit_entry(
        1_774_483_200, "s1", 30, 1_774_500_000, 50, 1_775_000_000,
    )];
    let opts = SlFormatOptions {
        tz: Some("UTC".to_string()),
        price_mode: PriceMode::Decimal,
        compact: false,
        color: false,
    };
    let result = format_sl_ratelimit_table(&entries, &opts);
    assert!(result.contains('┌'), "should use box-drawing top-left corner");
    assert!(result.contains('┘'), "should use box-drawing bottom-right corner");
    assert!(result.contains('│'), "should use box-drawing vertical bar");
}

// ─── Session table ────────────────────────────────────────────────────────────

#[test]
fn test_session_table_full_headers() {
    let sessions = vec![make_session_summary(
        "abc123", "/home/user/foo/bar", 0.50, 3_600_000, 1_800_000, 100, 50, Some(75), 2,
    )];
    let opts = SlFormatOptions {
        tz: Some("UTC".to_string()),
        price_mode: PriceMode::Decimal,
        compact: false,
        color: false,
    };
    let result = format_sl_session_table(&sessions, &opts);

    assert!(result.contains("Session"), "should contain Session");
    assert!(result.contains("Cost"), "should contain Cost");
    assert!(result.contains("Duration"), "should contain Duration");
    assert!(result.contains("API Time"), "should contain API Time");
    assert!(result.contains("Lines +/-"), "should contain Lines +/-");
    assert!(result.contains("Segs"), "should contain Segs");
    assert!(result.contains("5h%"), "should contain 5h%");
    assert!(result.contains("1w%"), "should contain 1w%");
}

#[test]
fn test_session_table_compact_headers() {
    let sessions = vec![make_session_summary(
        "abc123", "/home/user/foo/bar", 0.50, 3_600_000, 1_800_000, 100, 50, Some(75), 2,
    )];
    let opts = SlFormatOptions {
        tz: Some("UTC".to_string()),
        price_mode: PriceMode::Decimal,
        compact: true,
        color: false,
    };
    let result = format_sl_session_table(&sessions, &opts);

    assert!(result.contains("Session"), "should contain Session");
    assert!(result.contains("Cost"), "should contain Cost");
    assert!(result.contains("Segs"), "should contain Segs");
    assert!(result.contains("5h%"), "should contain 5h%");
    // Columns hidden in compact mode
    assert!(!result.contains("API Time"), "compact should not have API Time");
    assert!(!result.contains("1w%"), "compact should not have 1w%");
}

#[test]
fn test_session_table_duration_shown() {
    let sessions = vec![make_session_summary(
        "abc123", "/home/user/foo/bar", 0.50, 3_600_000, 1_800_000, 100, 50, Some(75), 2,
    )];
    let opts = SlFormatOptions {
        tz: Some("UTC".to_string()),
        price_mode: PriceMode::Decimal,
        compact: false,
        color: false,
    };
    let result = format_sl_session_table(&sessions, &opts);
    assert!(result.contains("1h 0m"), "should show duration formatted as hours and minutes");
}

#[test]
fn test_session_table_lines_format() {
    let sessions = vec![make_session_summary(
        "abc123", "/proj/a", 0.5, 1_000, 500, 42, 17, None, 1,
    )];
    let opts = SlFormatOptions {
        tz: Some("UTC".to_string()),
        price_mode: PriceMode::Decimal,
        compact: false,
        color: false,
    };
    let result = format_sl_session_table(&sessions, &opts);
    assert!(result.contains("+42 -17"), "should contain lines added/removed");
}

#[test]
fn test_session_table_peak_pct_shown() {
    let sessions = vec![make_session_summary(
        "abc123", "/proj/a", 0.5, 1_000, 500, 0, 0, None, 1,
    )];
    let opts = SlFormatOptions {
        tz: Some("UTC".to_string()),
        price_mode: PriceMode::Decimal,
        compact: false,
        color: false,
    };
    let result = format_sl_session_table(&sessions, &opts);
    // make_session_summary sets min/max_five_hour_pct=30, min/max_seven_day_pct=50
    assert!(result.contains("30%"), "should show peak 5h%");
    assert!(result.contains("50%"), "should show peak 1w%");
}

#[test]
fn test_session_table_segments_shown() {
    let sessions = vec![make_session_summary(
        "abc123", "/proj/a", 0.5, 1_000, 500, 0, 0, None, 3,
    )];
    let opts = SlFormatOptions {
        tz: Some("UTC".to_string()),
        price_mode: PriceMode::Decimal,
        compact: false,
        color: false,
    };
    let result = format_sl_session_table(&sessions, &opts);
    assert!(result.contains('3'), "should show segment count");
}

// ─── JSON output ──────────────────────────────────────────────────────────────

#[test]
fn test_json_ratelimit_structure() {
    let entries = vec![make_ratelimit_entry(
        1_774_483_200, "session-abc123", 30, 1_774_500_000, 50, 1_775_000_000,
    )];
    let meta = make_json_meta("ratelimit");
    let result = format_sl_json_ratelimit(&entries, &meta);
    let parsed: serde_json::Value =
        serde_json::from_str(&result).expect("should produce valid JSON");

    assert!(parsed["meta"].is_object(), "should have meta object");
    assert!(parsed["data"].is_array(), "should have data array");
    assert_eq!(parsed["data"].as_array().unwrap().len(), 1);
}

#[test]
fn test_json_ratelimit_meta_fields() {
    let entries = vec![];
    let meta = SlJsonMeta {
        source: "my-source".to_string(),
        file: "my-file.jsonl".to_string(),
        view: "ratelimit".to_string(),
        from: Some("2026-01-01".to_string()),
        to: Some("2026-03-26".to_string()),
        tz: Some("UTC".to_string()),
        generated_at: "2026-03-26T00:00:00Z".to_string(),
    };
    let result = format_sl_json_ratelimit(&entries, &meta);
    let parsed: serde_json::Value =
        serde_json::from_str(&result).expect("valid JSON");

    assert_eq!(parsed["meta"]["source"], "my-source");
    assert_eq!(parsed["meta"]["file"], "my-file.jsonl");
    assert_eq!(parsed["meta"]["view"], "ratelimit");
    assert_eq!(parsed["meta"]["from"], "2026-01-01");
    assert_eq!(parsed["meta"]["to"], "2026-03-26");
    assert_eq!(parsed["meta"]["tz"], "UTC");
    assert_eq!(parsed["meta"]["generatedAt"], "2026-03-26T00:00:00Z");
}

#[test]
fn test_json_ratelimit_null_from_to() {
    let entries = vec![];
    let meta = make_json_meta("ratelimit");
    let result = format_sl_json_ratelimit(&entries, &meta);
    let parsed: serde_json::Value = serde_json::from_str(&result).expect("valid JSON");
    assert!(parsed["meta"]["from"].is_null(), "from should be null when not set");
    assert!(parsed["meta"]["to"].is_null(), "to should be null when not set");
}

#[test]
fn test_json_sessions_has_totals() {
    let sessions = vec![
        make_session_summary("s1", "/proj/a", 0.5, 3_600_000, 1_800_000, 10, 5, None, 1),
        make_session_summary("s2", "/proj/b", 0.3, 1_800_000, 900_000, 5, 2, None, 1),
    ];
    let meta = make_json_meta("sessions");
    let result = format_sl_json_sessions(&sessions, &meta);
    let parsed: serde_json::Value = serde_json::from_str(&result).expect("valid JSON");

    assert!(parsed["totals"].is_object(), "should have totals object");
    assert_eq!(parsed["totals"]["sessionCount"], 2);

    let total_cost = parsed["totals"]["totalCost"].as_f64().unwrap();
    assert!(
        (total_cost - 0.8).abs() < 1e-9,
        "total_cost={} (expected 0.8)",
        total_cost
    );
}

#[test]
fn test_json_sessions_totals_duration() {
    let sessions = vec![
        make_session_summary("s1", "/proj/a", 0.1, 1_000, 500, 0, 0, None, 1),
        make_session_summary("s2", "/proj/b", 0.2, 2_000, 800, 0, 0, None, 1),
    ];
    let meta = make_json_meta("sessions");
    let result = format_sl_json_sessions(&sessions, &meta);
    let parsed: serde_json::Value = serde_json::from_str(&result).expect("valid JSON");

    assert_eq!(parsed["totals"]["totalDurationMs"], 3000);
    assert_eq!(parsed["totals"]["totalApiDurationMs"], 1300);
}

#[test]
fn test_json_sessions_data_array() {
    let sessions = vec![
        make_session_summary("s1", "/proj/a", 0.5, 1_000, 500, 0, 0, None, 1),
    ];
    let meta = make_json_meta("sessions");
    let result = format_sl_json_sessions(&sessions, &meta);
    let parsed: serde_json::Value = serde_json::from_str(&result).expect("valid JSON");

    assert!(parsed["data"].is_array());
    assert_eq!(parsed["data"].as_array().unwrap().len(), 1);
    assert_eq!(parsed["data"][0]["sessionId"], "s1");
}

#[test]
fn test_json_windows_structure() {
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
        five_hour_resets_at: None,
    }];
    let meta = make_json_meta("windows");
    let result = format_sl_json_windows(&windows, &meta);
    let parsed: serde_json::Value = serde_json::from_str(&result).expect("valid JSON");

    assert!(parsed["data"].is_array());
    assert_eq!(parsed["data"][0]["minFiveHourPct"], 45);
    assert_eq!(parsed["data"][0]["sessions"], 3);
}

#[test]
fn test_json_projects_structure() {
    let projects = vec![SlProjectSummary {
        project: "/home/user/foo".to_string(),
        total_cost: 2.5,
        total_duration_ms: 10_000,
        total_api_duration_ms: 5_000,
        session_count: 4,
        total_lines_added: 20,
        total_lines_removed: 8,
        min_five_hour_pct: Some(40),
        max_five_hour_pct: Some(40),
        min_seven_day_pct: Some(70),
        max_seven_day_pct: Some(70),
    }];
    let meta = make_json_meta("projects");
    let result = format_sl_json_projects(&projects, &meta);
    let parsed: serde_json::Value = serde_json::from_str(&result).expect("valid JSON");

    assert!(parsed["data"].is_array());
    assert_eq!(parsed["data"][0]["sessionCount"], 4);
}

#[test]
fn test_json_days_structure() {
    let days = vec![SlDaySummary {
        date: "2026-03-26".to_string(),
        total_cost: 1.5,
        session_count: 5,
        min_five_hour_pct: Some(60),
        max_five_hour_pct: Some(60),
        min_seven_day_pct: Some(80),
        max_seven_day_pct: Some(80),
        total_duration_ms: 7_200_000,
        total_api_duration_ms: 3_600_000,
        total_lines_added: 50,
        total_lines_removed: 20,
    }];
    let meta = make_json_meta("days");
    let result = format_sl_json_days(&days, &meta);
    let parsed: serde_json::Value = serde_json::from_str(&result).expect("valid JSON");

    assert!(parsed["data"].is_array());
    assert_eq!(parsed["data"][0]["date"], "2026-03-26");
    assert_eq!(parsed["data"][0]["sessionCount"], 5);
}

// ─── CSV formatters ───────────────────────────────────────────────────────────

#[test]
fn test_csv_ratelimit_header_row() {
    let entries = vec![];
    let result = format_sl_csv_ratelimit(&entries, Some("UTC"));
    let first_line = result.lines().next().expect("should have header line");
    assert_eq!(first_line, "Time,Cost,5h%,1w%,5h Resets,1w Resets,Session");
}

#[test]
fn test_csv_ratelimit_data_row() {
    let entries = vec![make_ratelimit_entry(
        1_774_483_200, "session-abc123", 30, 1_774_500_000, 50, 1_775_000_000,
    )];
    let result = format_sl_csv_ratelimit(&entries, Some("UTC"));
    let lines: Vec<&str> = result.lines().collect();
    assert_eq!(lines.len(), 2, "should have header + 1 data row");
    let data = lines[1];
    assert!(data.contains("30"), "should contain 5h%");
    assert!(data.contains("50"), "should contain 1w%");
    assert!(data.contains("session-abc123"), "should contain full session id");
}

#[test]
fn test_csv_ratelimit_time_format() {
    // 2026-03-26T04:40:00Z
    let entries = vec![make_ratelimit_entry(
        1_774_483_200, "s1", 30, 1_774_500_000, 50, 1_775_000_000,
    )];
    let result = format_sl_csv_ratelimit(&entries, Some("UTC"));
    let lines: Vec<&str> = result.lines().collect();
    // fmt_time gives "YYYY-MM-DD HH:MM"
    assert!(lines[1].starts_with("2026-03-26"), "time column should start with date");
}

#[test]
fn test_csv_sessions_header_row() {
    let sessions = vec![];
    let opts = SlFormatOptions {
        tz: Some("UTC".to_string()),
        price_mode: PriceMode::Decimal,
        compact: false,
        color: false,
    };
    let result = format_sl_csv_sessions(&sessions, &opts);
    let first_line = result.lines().next().expect("should have header line");
    assert!(first_line.contains("Session"), "header should contain Session");
    assert!(first_line.contains("Cost"), "header should contain Cost");
    assert!(first_line.contains("API Time"), "header should contain API Time");
    assert!(first_line.contains("Lines Added"), "header should contain Lines Added");
}

#[test]
fn test_csv_sessions_data_row() {
    let sessions = vec![make_session_summary(
        "session-xyz", "/proj/a", 1.234567, 3_600_000, 1_800_000, 42, 17, Some(60), 2,
    )];
    let opts = SlFormatOptions {
        tz: Some("UTC".to_string()),
        price_mode: PriceMode::Decimal,
        compact: false,
        color: false,
    };
    let result = format_sl_csv_sessions(&sessions, &opts);
    let lines: Vec<&str> = result.lines().collect();
    assert_eq!(lines.len(), 2, "should have header + 1 data row");
    let data = lines[1];
    assert!(data.contains("session-xyz"), "should contain session_id");
    assert!(data.contains("/proj/a"), "should contain project");
    assert!(data.contains("42"), "should contain lines_added");
    assert!(data.contains("17"), "should contain lines_removed");
    assert!(data.contains("60"), "should contain ctx_pct");
    assert!(data.contains('2'.to_string().as_str()), "should contain segments");
}

