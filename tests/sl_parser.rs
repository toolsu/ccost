use ccost::sl::{SlLoadOptions, load_sl_records};
use std::io::Write;
use tempfile::NamedTempFile;

// ─── Helpers ──────────────────────────────────────────────────────────────────

fn write_jsonl(lines: &[&str]) -> NamedTempFile {
    let mut f = NamedTempFile::new().expect("create temp file");
    for line in lines {
        writeln!(f, "{}", line).expect("write line");
    }
    f
}

/// A complete valid JSONL entry with all fields including rate_limits.
fn full_entry(ts: i64, session_id: &str, project_dir: &str, model_id: &str) -> String {
    full_entry_with_display(ts, session_id, project_dir, model_id, "Claude Sonnet")
}

/// Like full_entry but with a custom display_name for the model.
fn full_entry_with_display(
    ts: i64,
    session_id: &str,
    project_dir: &str,
    model_id: &str,
    display_name: &str,
) -> String {
    format!(
        r#"{{"ts": {ts}, "data": {{"session_id": "{session_id}", "workspace": {{"project_dir": "{project_dir}"}}, "model": {{"id": "{model_id}", "display_name": "{display_name}"}}, "version": "1.2.3", "cost": {{"total_cost_usd": 0.5, "total_duration_ms": 1000, "total_api_duration_ms": 500, "total_lines_added": 10, "total_lines_removed": 5}}, "context_window": {{"used_percentage": 2, "context_window_size": 1000000}}, "rate_limits": {{"five_hour": {{"used_percentage": 5, "resets_at": 1774497600}}, "seven_day": {{"used_percentage": 63, "resets_at": 1774605600}}}}}}}}"#
    )
}

/// An entry without rate_limits.
fn entry_no_rate_limits(ts: i64, session_id: &str, project_dir: &str, model_id: &str) -> String {
    format!(
        r#"{{"ts": {ts}, "data": {{"session_id": "{session_id}", "workspace": {{"project_dir": "{project_dir}"}}, "model": {{"id": "{model_id}", "display_name": "Claude Haiku"}}, "version": "1.0.0", "cost": {{"total_cost_usd": 0.1, "total_duration_ms": 200, "total_api_duration_ms": 100, "total_lines_added": 0, "total_lines_removed": 0}}, "context_window": {{"used_percentage": null, "context_window_size": 200000}}}}}}"#
    )
}

// ─── Basic parsing tests ──────────────────────────────────────────────────────

#[test]
fn test_basic_record_parsing() {
    // ts 1774481258 = 2026-03-26 (approx)
    let line = full_entry(1774481258, "sess-abc", "/home/user/project", "claude-sonnet-4-5");
    let f = write_jsonl(&[&line]);
    let opts = SlLoadOptions::default();
    let (records, skipped) = load_sl_records(f.path().to_str().unwrap(), &opts);

    assert_eq!(skipped, 0);
    assert_eq!(records.len(), 1);

    let r = &records[0];
    assert_eq!(r.session_id, "sess-abc");
    assert_eq!(r.project, "/home/user/project");
    assert_eq!(r.model_id, "claude-sonnet-4-5");
    assert_eq!(r.model_name, "Claude Sonnet");
    assert_eq!(r.version, "1.2.3");
    assert!((r.cost_usd - 0.5).abs() < 1e-9);
    assert_eq!(r.duration_ms, 1000);
    assert_eq!(r.api_duration_ms, 500);
    assert_eq!(r.lines_added, 10);
    assert_eq!(r.lines_removed, 5);
    assert_eq!(r.context_pct, Some(2));
    assert_eq!(r.context_size, 1000000);
    // rate_limits present
    assert_eq!(r.five_hour_pct, Some(5));
    assert!(r.five_hour_resets_at.is_some());
    assert_eq!(r.seven_day_pct, Some(63));
    assert!(r.seven_day_resets_at.is_some());
    // timestamp should correspond to unix ts 1774481258
    assert_eq!(r.ts.timestamp(), 1774481258);
}

#[test]
fn test_multiple_records() {
    let lines = vec![
        full_entry(1774481258, "sess-1", "/proj/a", "claude-opus-4"),
        full_entry(1774481300, "sess-2", "/proj/b", "claude-sonnet-4-5"),
        full_entry(1774481400, "sess-3", "/proj/c", "claude-haiku-4"),
    ];
    let line_refs: Vec<&str> = lines.iter().map(|s| s.as_str()).collect();
    let f = write_jsonl(&line_refs);
    let opts = SlLoadOptions::default();
    let (records, skipped) = load_sl_records(f.path().to_str().unwrap(), &opts);

    assert_eq!(skipped, 0);
    assert_eq!(records.len(), 3);
}

// ─── Records without rate_limits ─────────────────────────────────────────────

#[test]
fn test_record_without_rate_limits() {
    let line = entry_no_rate_limits(1774481258, "sess-xyz", "/home/user/myapp", "claude-haiku-4");
    let f = write_jsonl(&[&line]);
    let opts = SlLoadOptions::default();
    let (records, skipped) = load_sl_records(f.path().to_str().unwrap(), &opts);

    assert_eq!(skipped, 0);
    assert_eq!(records.len(), 1);

    let r = &records[0];
    assert_eq!(r.session_id, "sess-xyz");
    assert_eq!(r.model_name, "Claude Haiku");
    assert_eq!(r.context_pct, None); // used_percentage was null
    assert_eq!(r.context_size, 200000);
    assert_eq!(r.five_hour_pct, None);
    assert!(r.five_hour_resets_at.is_none());
    assert_eq!(r.seven_day_pct, None);
    assert!(r.seven_day_resets_at.is_none());
}

#[test]
fn test_mixed_records_with_and_without_rate_limits() {
    let lines = vec![
        full_entry(1774481258, "sess-1", "/proj/a", "claude-opus-4"),
        entry_no_rate_limits(1774481300, "sess-2", "/proj/b", "claude-haiku-4"),
    ];
    let line_refs: Vec<&str> = lines.iter().map(|s| s.as_str()).collect();
    let f = write_jsonl(&line_refs);
    let opts = SlLoadOptions::default();
    let (records, skipped) = load_sl_records(f.path().to_str().unwrap(), &opts);

    assert_eq!(skipped, 0);
    assert_eq!(records.len(), 2);
    assert!(records[0].five_hour_pct.is_some());
    assert!(records[1].five_hour_pct.is_none());
}

// ─── Malformed line skipping ──────────────────────────────────────────────────

#[test]
fn test_malformed_lines_skipped() {
    let good = full_entry(1774481258, "sess-good", "/proj/good", "claude-sonnet-4-5");
    let lines = vec![
        good.as_str(),
        "not valid json",
        r#"{"ts": 123}"#,                    // missing data
        r#"{"data": {}}"#,                    // missing ts
        "",                                    // empty line (not counted as skipped)
        r#"{"ts": 1774481258, "data": {"session_id": "x"}}"#, // missing required data fields
    ];
    let f = write_jsonl(&lines);
    let opts = SlLoadOptions::default();
    let (records, skipped) = load_sl_records(f.path().to_str().unwrap(), &opts);

    assert_eq!(records.len(), 1);
    assert!(skipped >= 3, "expected at least 3 skipped, got {}", skipped);
}

#[test]
fn test_all_malformed_returns_empty() {
    let f = write_jsonl(&["garbage line 1", "garbage line 2", "{bad json"]);
    let opts = SlLoadOptions::default();
    let (records, skipped) = load_sl_records(f.path().to_str().unwrap(), &opts);
    assert_eq!(records.len(), 0);
    assert_eq!(skipped, 3);
}

#[test]
fn test_empty_file() {
    let f = write_jsonl(&[]);
    let opts = SlLoadOptions::default();
    let (records, skipped) = load_sl_records(f.path().to_str().unwrap(), &opts);
    assert_eq!(records.len(), 0);
    assert_eq!(skipped, 0);
}

// ─── Session filter ───────────────────────────────────────────────────────────

#[test]
fn test_session_filter_case_insensitive() {
    let lines = vec![
        full_entry(1774481258, "ABCDEF-session", "/proj/a", "claude-sonnet-4-5"),
        full_entry(1774481300, "xyz-session", "/proj/b", "claude-sonnet-4-5"),
        full_entry(1774481400, "abcdef-OTHER", "/proj/c", "claude-sonnet-4-5"),
    ];
    let line_refs: Vec<&str> = lines.iter().map(|s| s.as_str()).collect();
    let f = write_jsonl(&line_refs);

    let opts = SlLoadOptions {
        session: Some("abcdef".to_string()),
        ..Default::default()
    };
    let (records, _) = load_sl_records(f.path().to_str().unwrap(), &opts);

    assert_eq!(records.len(), 2);
    assert!(records.iter().all(|r| r.session_id.to_lowercase().contains("abcdef")));
}

#[test]
fn test_session_filter_no_match() {
    let line = full_entry(1774481258, "sess-abc", "/proj/a", "claude-sonnet-4-5");
    let f = write_jsonl(&[&line]);
    let opts = SlLoadOptions {
        session: Some("zzz-nomatch".to_string()),
        ..Default::default()
    };
    let (records, _) = load_sl_records(f.path().to_str().unwrap(), &opts);
    assert_eq!(records.len(), 0);
}

// ─── Project filter ───────────────────────────────────────────────────────────

#[test]
fn test_project_filter_case_insensitive() {
    let lines = vec![
        full_entry(1774481258, "sess-1", "/home/user/MyProject", "claude-sonnet-4-5"),
        full_entry(1774481300, "sess-2", "/home/user/other", "claude-sonnet-4-5"),
        full_entry(1774481400, "sess-3", "/home/user/myproject-fork", "claude-sonnet-4-5"),
    ];
    let line_refs: Vec<&str> = lines.iter().map(|s| s.as_str()).collect();
    let f = write_jsonl(&line_refs);

    let opts = SlLoadOptions {
        project: Some("myproject".to_string()),
        ..Default::default()
    };
    let (records, _) = load_sl_records(f.path().to_str().unwrap(), &opts);

    assert_eq!(records.len(), 2);
}

// ─── Model filter ─────────────────────────────────────────────────────────────

#[test]
fn test_model_filter_matches_model_id() {
    // Use distinct display_names so "sonnet" only appears in one entry
    let lines = vec![
        full_entry_with_display(1774481258, "sess-1", "/proj/a", "claude-sonnet-4-5", "Claude Sonnet"),
        full_entry_with_display(1774481300, "sess-2", "/proj/b", "claude-opus-4", "Claude Opus"),
        full_entry_with_display(1774481400, "sess-3", "/proj/c", "claude-haiku-4", "Claude Haiku"),
    ];
    let line_refs: Vec<&str> = lines.iter().map(|s| s.as_str()).collect();
    let f = write_jsonl(&line_refs);

    let opts = SlLoadOptions {
        model: Some("sonnet".to_string()),
        ..Default::default()
    };
    let (records, _) = load_sl_records(f.path().to_str().unwrap(), &opts);

    assert_eq!(records.len(), 1);
    assert_eq!(records[0].model_id, "claude-sonnet-4-5");
}

#[test]
fn test_model_filter_matches_display_name() {
    // entry_no_rate_limits uses "Claude Haiku" as display_name; the model_id does not contain "haiku"
    // Use a model_id that won't match on its own to confirm the display_name is also searched
    let lines = vec![
        full_entry_with_display(1774481258, "sess-1", "/proj/a", "model-xyz-001", "Claude Haiku"),
        full_entry_with_display(1774481300, "sess-2", "/proj/b", "model-xyz-002", "Claude Opus"),
    ];
    let line_refs: Vec<&str> = lines.iter().map(|s| s.as_str()).collect();
    let f = write_jsonl(&line_refs);

    let opts = SlLoadOptions {
        model: Some("haiku".to_string()),
        ..Default::default()
    };
    let (records, _) = load_sl_records(f.path().to_str().unwrap(), &opts);

    assert_eq!(records.len(), 1);
    assert_eq!(records[0].model_name, "Claude Haiku");
}

// ─── Time range filtering ─────────────────────────────────────────────────────

// Timestamps used in time-filter tests (UTC midnight boundaries):
// 1774483200 = 2026-03-26T00:00:00Z
// 1774569600 = 2026-03-27T00:00:00Z
// 1774656000 = 2026-03-28T00:00:00Z

#[test]
fn test_from_filter_date_only_utc() {
    let lines = vec![
        full_entry(1774483200, "sess-1", "/proj/a", "claude-sonnet-4-5"), // 2026-03-26
        full_entry(1774569600, "sess-2", "/proj/b", "claude-sonnet-4-5"), // 2026-03-27
        full_entry(1774656000, "sess-3", "/proj/c", "claude-sonnet-4-5"), // 2026-03-28
    ];
    let line_refs: Vec<&str> = lines.iter().map(|s| s.as_str()).collect();
    let f = write_jsonl(&line_refs);

    let opts = SlLoadOptions {
        from: Some("2026-03-27".to_string()),
        tz: Some("UTC".to_string()),
        ..Default::default()
    };
    let (records, _) = load_sl_records(f.path().to_str().unwrap(), &opts);

    assert_eq!(records.len(), 2, "should keep sess-2 and sess-3");
    assert!(records.iter().all(|r| r.ts.timestamp() >= 1774569600));
}

#[test]
fn test_to_filter_date_only_utc() {
    let lines = vec![
        full_entry(1774483200, "sess-1", "/proj/a", "claude-sonnet-4-5"), // 2026-03-26
        full_entry(1774569600, "sess-2", "/proj/b", "claude-sonnet-4-5"), // 2026-03-27
        full_entry(1774656000, "sess-3", "/proj/c", "claude-sonnet-4-5"), // 2026-03-28
    ];
    let line_refs: Vec<&str> = lines.iter().map(|s| s.as_str()).collect();
    let f = write_jsonl(&line_refs);

    let opts = SlLoadOptions {
        to: Some("2026-03-27".to_string()),
        tz: Some("UTC".to_string()),
        ..Default::default()
    };
    let (records, _) = load_sl_records(f.path().to_str().unwrap(), &opts);

    assert_eq!(records.len(), 2, "should keep sess-1 and sess-2");
}

#[test]
fn test_from_to_filter_utc() {
    let lines = vec![
        full_entry(1774483200, "sess-1", "/proj/a", "claude-sonnet-4-5"), // 2026-03-26
        full_entry(1774569600, "sess-2", "/proj/b", "claude-sonnet-4-5"), // 2026-03-27
        full_entry(1774656000, "sess-3", "/proj/c", "claude-sonnet-4-5"), // 2026-03-28
    ];
    let line_refs: Vec<&str> = lines.iter().map(|s| s.as_str()).collect();
    let f = write_jsonl(&line_refs);

    let opts = SlLoadOptions {
        from: Some("2026-03-27".to_string()),
        to: Some("2026-03-27".to_string()),
        tz: Some("UTC".to_string()),
        ..Default::default()
    };
    let (records, _) = load_sl_records(f.path().to_str().unwrap(), &opts);

    assert_eq!(records.len(), 1);
    assert_eq!(records[0].session_id, "sess-2");
}

#[test]
fn test_from_filter_fixed_offset() {
    // UTC+8 (fixed offset):
    // 1774483200 = 2026-03-26T00:00:00Z = 2026-03-26T08:00:00+08:00 -> date 2026-03-26 (excluded by from=2026-03-27)
    // 1774569600 = 2026-03-27T00:00:00Z = 2026-03-27T08:00:00+08:00 -> date 2026-03-27 (included)
    let lines = vec![
        full_entry(1774483200, "sess-1", "/proj/a", "claude-sonnet-4-5"),
        full_entry(1774569600, "sess-2", "/proj/b", "claude-sonnet-4-5"),
    ];
    let line_refs: Vec<&str> = lines.iter().map(|s| s.as_str()).collect();
    let f = write_jsonl(&line_refs);

    let opts = SlLoadOptions {
        from: Some("2026-03-27".to_string()),
        tz: Some("+08:00".to_string()),
        ..Default::default()
    };
    let (records, _) = load_sl_records(f.path().to_str().unwrap(), &opts);
    // Only sess-2 has date >= 2026-03-27 in +08:00
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].session_id, "sess-2");
}

#[test]
fn test_from_filter_iana_timezone() {
    // Asia/Shanghai = UTC+8 (same as +08:00 for these dates, no DST)
    // 1774483200 = 2026-03-26T08:00:00+08:00 -> date 2026-03-26 (excluded by from=2026-03-27)
    // 1774569600 = 2026-03-27T08:00:00+08:00 -> date 2026-03-27 (included)
    let lines = vec![
        full_entry(1774483200, "sess-1", "/proj/a", "claude-sonnet-4-5"),
        full_entry(1774569600, "sess-2", "/proj/b", "claude-sonnet-4-5"),
    ];
    let line_refs: Vec<&str> = lines.iter().map(|s| s.as_str()).collect();
    let f = write_jsonl(&line_refs);

    let opts = SlLoadOptions {
        from: Some("2026-03-27".to_string()),
        tz: Some("Asia/Shanghai".to_string()),
        ..Default::default()
    };
    let (records, _) = load_sl_records(f.path().to_str().unwrap(), &opts);
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].session_id, "sess-2");
}

// ─── Combined filters ─────────────────────────────────────────────────────────

#[test]
fn test_combined_session_and_project_filter() {
    let lines = vec![
        full_entry(1774481258, "abc-session", "/home/user/myproject", "claude-sonnet-4-5"),
        full_entry(1774481300, "abc-session", "/home/user/other", "claude-sonnet-4-5"),
        full_entry(1774481400, "xyz-session", "/home/user/myproject", "claude-sonnet-4-5"),
    ];
    let line_refs: Vec<&str> = lines.iter().map(|s| s.as_str()).collect();
    let f = write_jsonl(&line_refs);

    let opts = SlLoadOptions {
        session: Some("abc".to_string()),
        project: Some("myproject".to_string()),
        ..Default::default()
    };
    let (records, _) = load_sl_records(f.path().to_str().unwrap(), &opts);

    assert_eq!(records.len(), 1);
    assert_eq!(records[0].session_id, "abc-session");
    assert_eq!(records[0].project, "/home/user/myproject");
}

// ─── File not found ───────────────────────────────────────────────────────────

#[test]
fn test_nonexistent_file_returns_empty() {
    let opts = SlLoadOptions::default();
    let (records, skipped) = load_sl_records("/nonexistent/path/file.jsonl", &opts);
    assert_eq!(records.len(), 0);
    assert_eq!(skipped, 0);
}
