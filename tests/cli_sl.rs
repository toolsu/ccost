use assert_cmd::Command;
use predicates::prelude::*;
use std::io::Write;
use tempfile::NamedTempFile;

// ─── Fixture helpers ──────────────────────────────────────────────────────────

/// Build a single statusline JSONL line.
///
/// Parameters:
/// - `ts`: Unix timestamp (seconds)
/// - `session`: session UUID string
/// - `cost`: total_cost_usd
/// - `dur_ms`: total_duration_ms
/// - `api_ms`: total_api_duration_ms
/// - `lines_add`: total_lines_added
/// - `lines_rem`: total_lines_removed
/// - `five_h`: five_hour used_percentage (or None to omit rate_limits entirely)
/// - `seven_d`: seven_day used_percentage
/// - `resets_5h`: five_hour resets_at unix timestamp
/// - `resets_7d`: seven_day resets_at unix timestamp
///
/// Rate limits must have all 4 fields (five_h, seven_d, resets_5h, resets_7d) or none.
fn make_sl_line(
    ts: i64,
    session: &str,
    cost: f64,
    dur_ms: u64,
    api_ms: u64,
    lines_add: u64,
    lines_rem: u64,
    five_h: Option<u8>,
    seven_d: Option<u8>,
    resets_5h: Option<i64>,
    resets_7d: Option<i64>,
) -> String {
    let rate_limits = match (five_h, seven_d, resets_5h, resets_7d) {
        (Some(fh), Some(sd), Some(r5), Some(r7)) => format!(
            r#","rate_limits":{{"five_hour":{{"used_percentage":{fh},"resets_at":{r5}}},"seven_day":{{"used_percentage":{sd},"resets_at":{r7}}}}}"#
        ),
        _ => String::new(),
    };

    format!(
        r#"{{"ts":{ts},"data":{{"session_id":"{session}","workspace":{{"project_dir":"/home/user/test-project","current_dir":"/home/user/test-project","added_dirs":[]}},"model":{{"id":"claude-opus-4-6[1m]","display_name":"Opus 4.6"}},"version":"2.1.84","cost":{{"total_cost_usd":{cost},"total_duration_ms":{dur_ms},"total_api_duration_ms":{api_ms},"total_lines_added":{lines_add},"total_lines_removed":{lines_rem}}},"context_window":{{"total_input_tokens":100,"total_output_tokens":50,"context_window_size":1000000,"current_usage":null,"used_percentage":2,"remaining_percentage":98}},"exceeds_200k_tokens":false{rate_limits}}}}}"#
    )
}

/// Create a temp file with test fixture data:
///
/// Session 1 ("sess-aaaa-1111-0000-000000000000"): 3 records, single segment.
///   cost 0 → 0.5 → 1.0
///   five_hour: 2%, 3%, 4%  seven_day: 63%  resets_at: 1774497600 / 1774605600
///
/// Session 2 ("sess-bbbb-2222-0000-000000000000"): 4 records with a segment reset.
///   Records 1-2: cost 0 → 2.0, five_hour 4%→5%, seven_day 63%→64%, resets_at=1774497600/1774605600
///   Reset: cost drops to 0, duration drops, new resets_at=1774515600 (different window)
///   Records 3-4: cost 0 → 0.5, five_hour 1%→2%, seven_day 64%
fn create_test_file() -> NamedTempFile {
    let sess1 = "sess-aaaa-1111-0000-000000000000";
    let sess2 = "sess-bbbb-2222-0000-000000000000";

    // Base timestamps (roughly 2026-03-26 in UTC)
    // 1774483200 = 2026-03-26T00:00:00Z
    let t = 1774483200_i64;

    let lines: Vec<String> = vec![
        // Session 1 – record 1 (cost=0, start of segment)
        make_sl_line(t + 100, sess1, 0.0, 5000, 2000, 0, 0,
                     Some(2), Some(63), Some(1774497600), Some(1774605600)),
        // Session 1 – record 2 (cost=0.5)
        make_sl_line(t + 200, sess1, 0.5, 15000, 6000, 5, 2,
                     Some(3), Some(63), Some(1774497600), Some(1774605600)),
        // Session 1 – record 3 (cost=1.0, final)
        make_sl_line(t + 300, sess1, 1.0, 30000, 12000, 10, 5,
                     Some(4), Some(63), Some(1774497600), Some(1774605600)),

        // Session 2 – record 1 (cost=0, start of first segment)
        make_sl_line(t + 400, sess2, 0.0, 3000, 1200, 0, 0,
                     Some(4), Some(63), Some(1774497600), Some(1774605600)),
        // Session 2 – record 2 (cost=2.0, end of first segment)
        make_sl_line(t + 500, sess2, 2.0, 40000, 16000, 20, 8,
                     Some(5), Some(64), Some(1774497600), Some(1774605600)),
        // Session 2 – reset: cost drops to 0, short duration → new segment
        make_sl_line(t + 600, sess2, 0.0, 1000, 400, 0, 0,
                     Some(1), Some(64), Some(1774515600), Some(1774605600)),
        // Session 2 – record 4 (cost=0.5, end of second segment)
        make_sl_line(t + 700, sess2, 0.5, 8000, 3200, 5, 2,
                     Some(2), Some(64), Some(1774515600), Some(1774605600)),
    ];

    let mut f = NamedTempFile::new().expect("create temp file");
    for line in &lines {
        writeln!(f, "{}", line).expect("write line");
    }
    f
}

// ─── Helper ───────────────────────────────────────────────────────────────────

fn sl_cmd() -> Command {
    Command::cargo_bin("ccost").unwrap()
}

// ─── Tests ────────────────────────────────────────────────────────────────────

/// 1. Default sl view should show session table (unified columns).
#[test]
fn test_sl_default_shows_5h() {
    let f = create_test_file();
    let path = f.path().to_str().unwrap();

    sl_cmd()
        .args(["sl", "--file", path])
        .assert()
        .success()
        .stdout(predicate::str::contains("5h Window"))
        .stdout(predicate::str::contains("Cost"))
        .stdout(predicate::str::contains("Duration"));
}

/// 1b. --per action shows rate-limit timeline with 5h% and 1w% columns.
#[test]
fn test_sl_per_action() {
    let f = create_test_file();
    let path = f.path().to_str().unwrap();

    sl_cmd()
        .args(["sl", "--file", path, "--per", "action"])
        .assert()
        .success()
        .stdout(predicate::str::contains("5h%"))
        .stdout(predicate::str::contains("1w%"));
}

/// 2. --per session shows session IDs and cumulative costs.
///    Session 1 total = 1.00 USD (single segment, max cost = 1.0)
///    Session 2 total = 2.50 USD (seg1 max=2.0 + seg2 max=0.5)
///
///    Note: the table formatter truncates session IDs to 8 chars.
///    "sess-aaaa-1111-..." → "sess-aaa"
///    "sess-bbbb-2222-..." → "sess-bbb"
#[test]
fn test_sl_per_session() {
    let f = create_test_file();
    let path = f.path().to_str().unwrap();

    sl_cmd()
        .args(["sl", "--file", path, "--per", "session", "--cost", "decimal"])
        .assert()
        .success()
        .stdout(predicate::str::contains("sess-aaa"))
        .stdout(predicate::str::contains("sess-bbb"))
        .stdout(predicate::str::contains("$1.00"))
        .stdout(predicate::str::contains("$2.50"));
}

/// 3. --per window shows "5h%" and "Est 5h Budg" column headers.
#[test]
fn test_sl_per_window() {
    let f = create_test_file();
    let path = f.path().to_str().unwrap();

    sl_cmd()
        .args(["sl", "--file", path, "--per", "5h", "--cost", "decimal"])
        .assert()
        .success()
        .stdout(predicate::str::contains("5h%"))
        .stdout(predicate::str::contains("Est 5h Budg"));
}

/// 4. --chart 5h prints "5-Hour Rate Limit" in output.
#[test]
fn test_sl_chart_5h() {
    let f = create_test_file();
    let path = f.path().to_str().unwrap();

    sl_cmd()
        .args(["sl", "--file", path, "--chart", "5h"])
        .assert()
        .success()
        .stdout(predicate::str::contains("5-Hour Rate Limit"));
}

/// 5. --per session --output json --filename /dev/stdout emits valid JSON
///    with meta.source = "ccost-sl" and a "data" array.
#[test]
fn test_sl_json_output() {
    let f = create_test_file();
    let path = f.path().to_str().unwrap();

    let output = sl_cmd()
        .args([
            "sl", "--file", path,
            "--per", "session",
            "--output", "json",
            "--filename", "/dev/stdout",
        ])
        .output()
        .expect("run command");

    assert!(output.status.success(), "command should succeed");

    let stdout_str = String::from_utf8(output.stdout).expect("valid utf8");
    let parsed: serde_json::Value =
        serde_json::from_str(&stdout_str).expect("stdout should be valid JSON");

    assert_eq!(
        parsed["meta"]["source"].as_str().unwrap_or(""),
        "ccost-sl",
        "meta.source should be 'ccost-sl'"
    );

    assert!(
        parsed["data"].is_array(),
        "JSON output should have a 'data' array"
    );
    assert!(
        !parsed["data"].as_array().unwrap().is_empty(),
        "data array should not be empty"
    );
}

/// 6. --file /nonexistent/path should fail with "not found" in stderr.
#[test]
fn test_sl_file_not_found() {
    sl_cmd()
        .args(["sl", "--file", "/nonexistent/path/statusline.jsonl"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("not found"));
}

/// 7. sl --help succeeds and stderr contains "statusline".
#[test]
fn test_sl_help() {
    sl_cmd()
        .args(["sl", "--help"])
        .assert()
        .success()
        .stderr(predicate::str::contains("statusline"));
}

/// 8. --per invalid should fail with "invalid" in stderr.
#[test]
fn test_sl_invalid_per() {
    let f = create_test_file();
    let path = f.path().to_str().unwrap();

    sl_cmd()
        .args(["sl", "--file", path, "--per", "invalid"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("invalid"));
}

/// 9. --session aaaa filter: only session 1 data should appear.
///
///    Note: the table formatter truncates session IDs to 8 chars.
///    "sess-aaaa-1111-..." → "sess-aaa"
///    "sess-bbbb-2222-..." → "sess-bbb"
#[test]
fn test_sl_session_filter() {
    let f = create_test_file();
    let path = f.path().to_str().unwrap();

    sl_cmd()
        .args([
            "sl", "--file", path,
            "--per", "session",
            "--session", "aaaa",
            "--cost", "decimal",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("sess-aaa"))
        .stdout(predicate::str::contains("$1.00"))
        // session 2 should NOT appear
        .stdout(predicate::str::contains("sess-bbb").not());
}

/// 10. --per day shows "Date" and "Sess" column headers.
#[test]
fn test_sl_per_day() {
    let f = create_test_file();
    let path = f.path().to_str().unwrap();

    sl_cmd()
        .args(["sl", "--file", path, "--per", "day"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Date"))
        .stdout(predicate::str::contains("Sess"));
}

/// 11. --per project shows "Project" and "Sess" column headers.
#[test]
fn test_sl_per_project() {
    let f = create_test_file();
    let path = f.path().to_str().unwrap();

    sl_cmd()
        .args(["sl", "--file", path, "--per", "project"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Project"))
        .stdout(predicate::str::contains("Sess"));
}

/// 12. --per 1h shows "1h Window", "5h Resets", and "Est 5h Budg" columns.
#[test]
fn test_sl_per_1h() {
    let f = create_test_file();
    let path = f.path().to_str().unwrap();

    sl_cmd()
        .args(["sl", "--file", path, "--per", "1h", "--table", "full"])
        .assert()
        .success()
        .stdout(predicate::str::contains("1h Window"))
        .stdout(predicate::str::contains("5h Resets"))
        .stdout(predicate::str::contains("Est 5h Budg"));
}

/// 13. --per 1w shows "1w Window" and "Est 1w Budg".
#[test]
fn test_sl_per_1w() {
    let f = create_test_file();
    let path = f.path().to_str().unwrap();

    sl_cmd()
        .args(["sl", "--file", path, "--per", "1w"])
        .assert()
        .success()
        .stdout(predicate::str::contains("1w Window"))
        .stdout(predicate::str::contains("Est 1w Budg"));
}

/// 14. --per action shows Cost column.
#[test]
fn test_sl_per_action_has_cost() {
    let f = create_test_file();
    let path = f.path().to_str().unwrap();

    sl_cmd()
        .args(["sl", "--file", path, "--per", "action"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Cost"))
        .stdout(predicate::str::contains("5h%"));
}

/// 15. All tables have a TOTAL row.
#[test]
fn test_sl_total_row_session() {
    let f = create_test_file();
    let path = f.path().to_str().unwrap();

    sl_cmd()
        .args(["sl", "--file", path, "--per", "session"])
        .assert()
        .success()
        .stdout(predicate::str::contains("TOTAL"));
}

#[test]
fn test_sl_total_row_5h() {
    let f = create_test_file();
    let path = f.path().to_str().unwrap();

    sl_cmd()
        .args(["sl", "--file", path, "--per", "5h"])
        .assert()
        .success()
        .stdout(predicate::str::contains("TOTAL"));
}

#[test]
fn test_sl_total_row_action() {
    let f = create_test_file();
    let path = f.path().to_str().unwrap();

    sl_cmd()
        .args(["sl", "--file", path, "--per", "action"])
        .assert()
        .success()
        .stdout(predicate::str::contains("TOTAL"));
}

/// 16. --nopromo flag is accepted.
#[test]
fn test_sl_nopromo_flag() {
    let f = create_test_file();
    let path = f.path().to_str().unwrap();

    sl_cmd()
        .args(["sl", "--file", path, "--per", "5h", "--nopromo"])
        .assert()
        .success()
        .stdout(predicate::str::contains("5h Window"));
}

/// 17. --output html produces full HTML page matching main CLI template.
#[test]
fn test_sl_output_html() {
    let f = create_test_file();
    let path = f.path().to_str().unwrap();

    let output = sl_cmd()
        .args(["sl", "--file", path, "--per", "5h", "--output", "html", "--filename", "/dev/stdout"])
        .output()
        .expect("run command");
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("<!DOCTYPE html>"), "should be full HTML page");
    assert!(stdout.contains("ccost report"), "should have title");
    assert!(stdout.contains("<style>"), "should have embedded CSS");
    assert!(stdout.contains("<script>"), "should have embedded JS");
    assert!(stdout.contains("background: #1a1816"), "should have dark theme CSS");
    assert!(stdout.contains("class=\"sortable\""), "should have sortable columns");
    assert!(stdout.contains("<thead>"), "should contain <thead>");
    assert!(stdout.contains("<tfoot>"), "should contain <tfoot>");
    assert!(stdout.contains("class=\"totals totals-main\""), "should have totals class");
    assert!(stdout.contains("TOTAL"), "should contain TOTAL in tfoot");
}

/// 18. --output markdown produces valid Markdown table.
#[test]
fn test_sl_output_markdown() {
    let f = create_test_file();
    let path = f.path().to_str().unwrap();

    let output = sl_cmd()
        .args(["sl", "--file", path, "--per", "session", "--output", "markdown", "--filename", "/dev/stdout"])
        .output()
        .expect("run command");
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("| Session |"), "should have markdown header");
    assert!(stdout.contains("Segments"), "should use full 'Segments' not 'Segs'");
    assert!(stdout.contains("| :--- |"), "should have alignment row");
    assert!(stdout.contains("**TOTAL**"), "should have bold TOTAL");
}

/// 19. --output tsv produces tab-separated output.
#[test]
fn test_sl_output_tsv() {
    let f = create_test_file();
    let path = f.path().to_str().unwrap();

    let output = sl_cmd()
        .args(["sl", "--file", path, "--per", "day", "--output", "tsv", "--filename", "/dev/stdout"])
        .output()
        .expect("run command");
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    let first_line = stdout.lines().next().unwrap();
    assert!(first_line.contains('\t'), "TSV should be tab-separated");
    assert!(first_line.contains("Date"), "TSV header should contain Date");
    assert!(stdout.contains("TOTAL"), "TSV should have TOTAL row");
}

/// 20. --output invalid format should fail.
#[test]
fn test_sl_output_invalid_format() {
    let f = create_test_file();
    let path = f.path().to_str().unwrap();

    sl_cmd()
        .args(["sl", "--file", path, "--output", "xml"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("invalid format"));
}

/// 21. --per 1h --output html works with 5h Resets column.
#[test]
fn test_sl_1h_output_html() {
    let f = create_test_file();
    let path = f.path().to_str().unwrap();

    let output = sl_cmd()
        .args(["sl", "--file", path, "--per", "1h", "--output", "html", "--filename", "/dev/stdout"])
        .output()
        .expect("run command");
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("5h Resets"), "1h HTML should have 5h Resets column");
    assert!(stdout.contains("Est 5h Budget"), "1h HTML should have full 'Est 5h Budget'");
    assert!(stdout.contains("Sessions"), "1h HTML should use full 'Sessions' not 'Sess'");
}
