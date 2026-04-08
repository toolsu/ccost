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
#[allow(clippy::too_many_arguments)]
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
        make_sl_line(
            t + 100,
            sess1,
            0.0,
            5000,
            2000,
            0,
            0,
            Some(2),
            Some(63),
            Some(1774497600),
            Some(1774605600),
        ),
        // Session 1 – record 2 (cost=0.5)
        make_sl_line(
            t + 200,
            sess1,
            0.5,
            15000,
            6000,
            5,
            2,
            Some(3),
            Some(63),
            Some(1774497600),
            Some(1774605600),
        ),
        // Session 1 – record 3 (cost=1.0, final)
        make_sl_line(
            t + 300,
            sess1,
            1.0,
            30000,
            12000,
            10,
            5,
            Some(4),
            Some(63),
            Some(1774497600),
            Some(1774605600),
        ),
        // Session 2 – record 1 (cost=0, start of first segment)
        make_sl_line(
            t + 400,
            sess2,
            0.0,
            3000,
            1200,
            0,
            0,
            Some(4),
            Some(63),
            Some(1774497600),
            Some(1774605600),
        ),
        // Session 2 – record 2 (cost=2.0, end of first segment)
        make_sl_line(
            t + 500,
            sess2,
            2.0,
            40000,
            16000,
            20,
            8,
            Some(5),
            Some(64),
            Some(1774497600),
            Some(1774605600),
        ),
        // Session 2 – reset: cost drops to 0, short duration → new segment
        make_sl_line(
            t + 600,
            sess2,
            0.0,
            1000,
            400,
            0,
            0,
            Some(1),
            Some(64),
            Some(1774515600),
            Some(1774605600),
        ),
        // Session 2 – record 4 (cost=0.5, end of second segment)
        make_sl_line(
            t + 700,
            sess2,
            0.5,
            8000,
            3200,
            5,
            2,
            Some(2),
            Some(64),
            Some(1774515600),
            Some(1774605600),
        ),
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
        .args([
            "sl", "--file", path, "--per", "session", "--cost", "decimal",
        ])
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

/// 5. --per session --output json --filename - emits valid JSON
///    with meta.source = "ccost-sl" and a "data" array.
#[test]
fn test_sl_json_output() {
    let f = create_test_file();
    let path = f.path().to_str().unwrap();

    let output = sl_cmd()
        .args([
            "sl",
            "--file",
            path,
            "--per",
            "session",
            "--output",
            "json",
            "--filename",
            "-",
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
            "sl",
            "--file",
            path,
            "--per",
            "session",
            "--session",
            "aaaa",
            "--cost",
            "decimal",
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
        .args([
            "sl",
            "--file",
            path,
            "--per",
            "5h",
            "--output",
            "html",
            "--filename",
            "-",
        ])
        .output()
        .expect("run command");
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(
        stdout.contains("<!DOCTYPE html>"),
        "should be full HTML page"
    );
    assert!(stdout.contains("ccost report"), "should have title");
    assert!(stdout.contains("<style>"), "should have embedded CSS");
    assert!(stdout.contains("<script>"), "should have embedded JS");
    assert!(
        stdout.contains("background: #1a1816"),
        "should have dark theme CSS"
    );
    assert!(
        stdout.contains("class=\"sortable\""),
        "should have sortable columns"
    );
    assert!(stdout.contains("<thead>"), "should contain <thead>");
    assert!(stdout.contains("<tfoot>"), "should contain <tfoot>");
    assert!(
        stdout.contains("class=\"totals totals-main\""),
        "should have totals class"
    );
    assert!(stdout.contains("TOTAL"), "should contain TOTAL in tfoot");
}

/// 18. --output markdown produces valid Markdown table.
#[test]
fn test_sl_output_markdown() {
    let f = create_test_file();
    let path = f.path().to_str().unwrap();

    let output = sl_cmd()
        .args([
            "sl",
            "--file",
            path,
            "--per",
            "session",
            "--output",
            "markdown",
            "--filename",
            "-",
        ])
        .output()
        .expect("run command");
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(
        stdout.contains("| Session |"),
        "should have markdown header"
    );
    assert!(
        stdout.contains("Segments"),
        "should use full 'Segments' not 'Segs'"
    );
    assert!(stdout.contains("| :--- |"), "should have alignment row");
    assert!(stdout.contains("**TOTAL**"), "should have bold TOTAL");
}

/// 19. --output tsv produces tab-separated output.
#[test]
fn test_sl_output_tsv() {
    let f = create_test_file();
    let path = f.path().to_str().unwrap();

    let output = sl_cmd()
        .args([
            "sl",
            "--file",
            path,
            "--per",
            "day",
            "--output",
            "tsv",
            "--filename",
            "-",
        ])
        .output()
        .expect("run command");
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    let first_line = stdout.lines().next().unwrap();
    assert!(first_line.contains('\t'), "TSV should be tab-separated");
    assert!(
        first_line.contains("Date"),
        "TSV header should contain Date"
    );
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
        .args([
            "sl",
            "--file",
            path,
            "--per",
            "1h",
            "--output",
            "html",
            "--filename",
            "-",
        ])
        .output()
        .expect("run command");
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(
        stdout.contains("5h Resets"),
        "1h HTML should have 5h Resets column"
    );
    assert!(
        stdout.contains("Est 5h Budget"),
        "1h HTML should have full 'Est 5h Budget'"
    );
    assert!(
        stdout.contains("Sessions"),
        "1h HTML should use full 'Sessions' not 'Sess'"
    );
}

/// 22. --order desc reverses row order (first window should come last).
#[test]
fn test_sl_order_desc() {
    let f = create_test_file();
    let path = f.path().to_str().unwrap();

    let asc_output = sl_cmd()
        .args(["sl", "--file", path, "--per", "5h", "--order", "asc"])
        .output()
        .expect("run asc");
    let desc_output = sl_cmd()
        .args(["sl", "--file", path, "--per", "5h", "--order", "desc"])
        .output()
        .expect("run desc");

    let asc_str = String::from_utf8(asc_output.stdout).unwrap();
    let desc_str = String::from_utf8(desc_output.stdout).unwrap();

    // Both should succeed and contain data
    assert!(asc_output.status.success());
    assert!(desc_output.status.success());

    // They should differ (rows are reversed, TOTAL stays at bottom)
    assert_ne!(
        asc_str, desc_str,
        "asc and desc should produce different output"
    );
}

/// 23. --chart with --output json should error.
#[test]
fn test_sl_chart_output_conflict() {
    let f = create_test_file();
    let path = f.path().to_str().unwrap();

    sl_cmd()
        .args(["sl", "--file", path, "--chart", "5h", "--output", "json"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("--chart only supports txt"));
}

/// 24. --per action TOTAL cost accumulates correctly even when pct is unchanged.
///     Regression test: skipped records (same pct pair) must not lose cost delta.
///     Fixture: 3 records, costs 0→1→2, pct 10%→10%→11%.
///     Record 2 is skipped (same pct as record 1). Record 3's delta should be $2 (not $1).
///     TOTAL should be $2.00.
#[test]
fn test_sl_action_cost_skipped_pct() {
    let sess = "sess-test-skip-0000-000000000000";
    let t = 1774483200_i64;
    let r5 = 1774497600_i64;
    let r7 = 1774605600_i64;

    let lines: Vec<String> = vec![
        make_sl_line(
            t + 100,
            sess,
            0.0,
            1000,
            400,
            0,
            0,
            Some(10),
            Some(60),
            Some(r5),
            Some(r7),
        ),
        // Same pct (10%, 60%) — will be skipped by dedup, cost=1.0
        make_sl_line(
            t + 200,
            sess,
            1.0,
            2000,
            800,
            5,
            2,
            Some(10),
            Some(60),
            Some(r5),
            Some(r7),
        ),
        // Pct changes to (11%, 60%) — should accumulate full delta $2.0
        make_sl_line(
            t + 300,
            sess,
            2.0,
            3000,
            1200,
            10,
            5,
            Some(11),
            Some(60),
            Some(r5),
            Some(r7),
        ),
    ];

    let mut f = NamedTempFile::new().expect("create temp file");
    for line in &lines {
        writeln!(f, "{}", line).expect("write line");
    }
    let path = f.path().to_str().unwrap();

    let output = sl_cmd()
        .args(["sl", "--file", path, "--per", "action", "--cost", "decimal"])
        .output()
        .expect("run command");
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();

    let total_line = stdout.lines().find(|l| l.contains("TOTAL")).unwrap();
    assert!(
        total_line.contains("$2.00"),
        "TOTAL should be $2.00 (skipped record's cost must accumulate into next action). Got: {}",
        total_line
    );
}

/// 25. --cost-diff table should contain Cost(SL), Cost(LiteLLM), and TOTAL.
#[test]
fn test_sl_cost_diff_table() {
    let f = create_test_file();
    let path = f.path().to_str().unwrap();

    sl_cmd()
        .args(["sl", "--file", path, "--per", "session", "--cost-diff"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Cost(SL)"))
        .stdout(predicate::str::contains("Cost(LiteLLM)"))
        .stdout(predicate::str::contains("Diff"))
        .stdout(predicate::str::contains("TOTAL"));
}

/// 26. --cost-diff --output json should have meta/data structure.
#[test]
fn test_sl_cost_diff_json() {
    let f = create_test_file();
    let path = f.path().to_str().unwrap();

    let output = sl_cmd()
        .args([
            "sl",
            "--file",
            path,
            "--per",
            "session",
            "--cost-diff",
            "--output",
            "json",
            "--filename",
            "-",
        ])
        .output()
        .expect("run command");
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&stdout).expect("should be valid JSON");
    assert_eq!(
        parsed["meta"]["source"].as_str().unwrap_or(""),
        "ccost-sl",
        "should have meta.source = ccost-sl"
    );
    assert!(parsed["data"].is_array(), "should have data array");
    assert!(parsed["totals"].is_object(), "should have totals object");
}

/// 27b. --cost-diff with non-session --per should fail.
#[test]
fn test_cost_diff_requires_per_session() {
    let f = create_test_file();
    let path = f.path().to_str().unwrap();

    sl_cmd()
        .args(["sl", "--file", path, "--cost-diff", "--per", "day"])
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "--cost-diff requires --per session",
        ));
}

/// 27. --per action TOTAL with the standard test fixture.
#[test]
fn test_sl_action_cost_total() {
    let f = create_test_file();
    let path = f.path().to_str().unwrap();

    let output = sl_cmd()
        .args(["sl", "--file", path, "--per", "action", "--cost", "decimal"])
        .output()
        .expect("run command");
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();

    let total_line = stdout.lines().find(|l| l.contains("TOTAL")).unwrap();
    assert!(
        total_line.contains("$3.50"),
        "TOTAL cost should be $3.50 (sess1=$1.00 + sess2 seg1=$2.00 + sess2 seg2=$0.50). Got: {}",
        total_line
    );
}

// ─── 28. --per 1h --output json ───────────────────────────────────────────────

#[test]
fn test_sl_per_1h_json() {
    let f = create_test_file();
    let path = f.path().to_str().unwrap();

    let output = sl_cmd()
        .args([
            "sl",
            "--file",
            path,
            "--per",
            "1h",
            "--output",
            "json",
            "--filename",
            "-",
        ])
        .output()
        .expect("run command");

    assert!(
        output.status.success(),
        "sl --per 1h --output json should succeed"
    );
    let stdout = String::from_utf8(output.stdout).unwrap();
    let parsed: serde_json::Value =
        serde_json::from_str(&stdout).expect("stdout should be valid JSON");

    assert_eq!(
        parsed["meta"]["source"].as_str().unwrap_or(""),
        "ccost-sl",
        "meta.source should be 'ccost-sl'"
    );
    assert_eq!(
        parsed["meta"]["view"].as_str().unwrap_or(""),
        "1h",
        "meta.view should be '1h'"
    );
    assert!(
        parsed["data"].is_array(),
        "JSON output should have 'data' array"
    );
}

// ─── 29. --per project --output json ─────────────────────────────────────────

#[test]
fn test_sl_per_project_json() {
    let f = create_test_file();
    let path = f.path().to_str().unwrap();

    let output = sl_cmd()
        .args([
            "sl",
            "--file",
            path,
            "--per",
            "project",
            "--output",
            "json",
            "--filename",
            "-",
        ])
        .output()
        .expect("run command");

    assert!(
        output.status.success(),
        "sl --per project --output json should succeed"
    );
    let stdout = String::from_utf8(output.stdout).unwrap();
    let parsed: serde_json::Value =
        serde_json::from_str(&stdout).expect("stdout should be valid JSON");

    assert_eq!(
        parsed["meta"]["view"].as_str().unwrap_or(""),
        "project",
        "meta.view should be 'project'"
    );
    assert!(
        parsed["data"].is_array(),
        "JSON output should have 'data' array"
    );
    assert!(
        !parsed["data"].as_array().unwrap().is_empty(),
        "data array should not be empty"
    );
}

// ─── 30. --per day --output json ──────────────────────────────────────────────

#[test]
fn test_sl_per_day_json() {
    let f = create_test_file();
    let path = f.path().to_str().unwrap();

    let output = sl_cmd()
        .args([
            "sl",
            "--file",
            path,
            "--per",
            "day",
            "--output",
            "json",
            "--filename",
            "-",
        ])
        .output()
        .expect("run command");

    assert!(
        output.status.success(),
        "sl --per day --output json should succeed"
    );
    let stdout = String::from_utf8(output.stdout).unwrap();
    let parsed: serde_json::Value =
        serde_json::from_str(&stdout).expect("stdout should be valid JSON");

    assert_eq!(
        parsed["meta"]["view"].as_str().unwrap_or(""),
        "day",
        "meta.view should be 'day'"
    );
    assert!(
        parsed["data"].is_array(),
        "JSON output should have 'data' array"
    );
    assert!(
        !parsed["data"].as_array().unwrap().is_empty(),
        "data array should not be empty"
    );
}

// ─── 31. --per session --table compact ───────────────────────────────────────

#[test]
fn test_sl_per_session_compact() {
    let f = create_test_file();
    let path = f.path().to_str().unwrap();

    sl_cmd()
        .args([
            "sl", "--file", path, "--per", "session", "--table", "compact",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("TOTAL"));
}

// ─── 32. --per action --table compact ────────────────────────────────────────

#[test]
fn test_sl_per_action_compact() {
    let f = create_test_file();
    let path = f.path().to_str().unwrap();

    sl_cmd()
        .args([
            "sl", "--file", path, "--per", "action", "--table", "compact",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("TOTAL"))
        .stdout(predicate::str::contains("5h%"));
}

// ─── 33. --output csv ─────────────────────────────────────────────────────────

#[test]
fn test_sl_output_csv() {
    let f = create_test_file();
    let path = f.path().to_str().unwrap();

    let output = sl_cmd()
        .args([
            "sl",
            "--file",
            path,
            "--per",
            "session",
            "--output",
            "csv",
            "--filename",
            "-",
        ])
        .output()
        .expect("run command");

    assert!(output.status.success(), "sl --output csv should succeed");
    let stdout = String::from_utf8(output.stdout).unwrap();
    let first_line = stdout
        .lines()
        .next()
        .expect("should have at least one line");
    assert!(
        first_line.contains(','),
        "CSV output should be comma-separated"
    );
    assert!(
        first_line.contains("Session"),
        "CSV header should contain 'Session'"
    );
    assert!(stdout.contains("TOTAL"), "CSV should have TOTAL row");
}

// ─── 34. --per 5h --nopromo ───────────────────────────────────────────────────

#[test]
fn test_sl_nopromo_with_5h() {
    let f = create_test_file();
    let path = f.path().to_str().unwrap();

    sl_cmd()
        .args(["sl", "--file", path, "--per", "5h", "--nopromo"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Est 5h Budg"))
        .stdout(predicate::str::contains("TOTAL"));
}

// ─── 35. --tz UTC ─────────────────────────────────────────────────────────────

#[test]
fn test_sl_timezone_utc() {
    let f = create_test_file();
    let path = f.path().to_str().unwrap();

    sl_cmd()
        .args(["sl", "--file", path, "--per", "session", "--tz", "UTC"])
        .assert()
        .success()
        .stdout(predicate::str::contains("TOTAL"));
}

// ─── 36. --model filter ───────────────────────────────────────────────────────

#[test]
fn test_sl_model_filter() {
    // Create a file with two sessions using different models
    let sess1 = "sess-model-filter-0000000000000a";
    let sess2 = "sess-model-filter-0000000000000b";
    let t = 1774483200_i64;

    // sess1 uses default model (opus-4-6); sess2 uses a different model id
    // Note: make_sl_line hardcodes model as "claude-opus-4-6[1m]"; we cannot override it here.
    // We filter by "opus" which matches both sessions — filter only confirms command succeeds.
    // Instead use a session filter to show model filter path is exercised.
    let lines: Vec<String> = vec![
        make_sl_line(
            t + 10,
            sess1,
            0.5,
            10000,
            4000,
            5,
            2,
            Some(2),
            Some(50),
            Some(1774497600),
            Some(1774605600),
        ),
        make_sl_line(
            t + 20,
            sess2,
            1.0,
            20000,
            8000,
            10,
            4,
            Some(3),
            Some(51),
            Some(1774497600),
            Some(1774605600),
        ),
    ];

    let mut f = NamedTempFile::new().expect("create temp file");
    for line in &lines {
        writeln!(f, "{}", line).expect("write line");
    }
    let path = f.path().to_str().unwrap();

    // Filter by "opus" — both sessions have "opus" in model name so both appear
    let output = sl_cmd()
        .args([
            "sl", "--file", path, "--per", "session", "--model", "opus", "--cost", "decimal",
        ])
        .output()
        .expect("run command");

    assert!(output.status.success(), "sl --model filter should succeed");
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("TOTAL"), "should have TOTAL row");
}

// ─── 37. --cost decimal mode ──────────────────────────────────────────────────

#[test]
fn test_sl_cost_decimal() {
    let f = create_test_file();
    let path = f.path().to_str().unwrap();

    sl_cmd()
        .args([
            "sl", "--file", path, "--per", "session", "--cost", "decimal",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("$"))
        .stdout(predicate::str::contains("."))
        .stdout(predicate::str::contains("TOTAL"));
}

// ─── 38. --output markdown --per session ──────────────────────────────────────

#[test]
fn test_sl_output_markdown_per_session() {
    let f = create_test_file();
    let path = f.path().to_str().unwrap();

    let output = sl_cmd()
        .args([
            "sl",
            "--file",
            path,
            "--per",
            "session",
            "--output",
            "markdown",
            "--filename",
            "-",
        ])
        .output()
        .expect("run command");

    assert!(
        output.status.success(),
        "sl --output markdown should succeed"
    );
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(
        stdout.contains("| Session |"),
        "markdown output should have Session header"
    );
    assert!(
        stdout.contains("| :--- |"),
        "markdown output should have alignment row"
    );
    assert!(
        stdout.contains("**TOTAL**"),
        "markdown output should have bold TOTAL"
    );
}

// ─── 39. --cost-diff --output json ────────────────────────────────────────────

/// Re-tests the cost-diff JSON path (already covered by test_sl_cost_diff_json above)
/// but from a different angle: validate individual field presence in each data row.
#[test]
fn test_sl_cost_diff_json_fields() {
    let f = create_test_file();
    let path = f.path().to_str().unwrap();

    let output = sl_cmd()
        .args([
            "sl",
            "--file",
            path,
            "--per",
            "session",
            "--cost-diff",
            "--output",
            "json",
            "--filename",
            "-",
        ])
        .output()
        .expect("run command");

    assert!(output.status.success(), "cost-diff JSON should succeed");
    let stdout = String::from_utf8(output.stdout).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&stdout).expect("should be valid JSON");

    assert!(parsed["data"].is_array(), "should have data array");
    assert!(parsed["totals"].is_object(), "should have totals object");
    // Totals for cost-diff use camelCase keys: totalSlCost and matchedCount
    let totals = &parsed["totals"];
    assert!(
        totals.get("totalSlCost").is_some(),
        "totals should have 'totalSlCost' field"
    );
    assert!(
        totals.get("matchedCount").is_some(),
        "totals should have 'matchedCount' field"
    );
}

// ─── 40. --per 1w --output json ───────────────────────────────────────────────

#[test]
fn test_sl_per_1w_json() {
    let f = create_test_file();
    let path = f.path().to_str().unwrap();

    let output = sl_cmd()
        .args([
            "sl",
            "--file",
            path,
            "--per",
            "1w",
            "--output",
            "json",
            "--filename",
            "-",
        ])
        .output()
        .expect("run command");

    assert!(
        output.status.success(),
        "sl --per 1w --output json should succeed"
    );
    let stdout = String::from_utf8(output.stdout).unwrap();
    let parsed: serde_json::Value =
        serde_json::from_str(&stdout).expect("stdout should be valid JSON");

    assert_eq!(
        parsed["meta"]["view"].as_str().unwrap_or(""),
        "1w",
        "meta.view should be '1w'"
    );
    assert!(
        parsed["data"].is_array(),
        "JSON output should have 'data' array"
    );
}

// ─── 41. --per action --output json ───────────────────────────────────────────

#[test]
fn test_sl_per_action_json() {
    let f = create_test_file();
    let path = f.path().to_str().unwrap();

    let output = sl_cmd()
        .args([
            "sl",
            "--file",
            path,
            "--per",
            "action",
            "--output",
            "json",
            "--filename",
            "-",
        ])
        .output()
        .expect("run command");

    assert!(
        output.status.success(),
        "sl --per action --output json should succeed"
    );
    let stdout = String::from_utf8(output.stdout).unwrap();
    let parsed: serde_json::Value =
        serde_json::from_str(&stdout).expect("stdout should be valid JSON");

    assert_eq!(
        parsed["meta"]["view"].as_str().unwrap_or(""),
        "action",
        "meta.view should be 'action'"
    );
    assert!(
        parsed["data"].is_array(),
        "JSON output should have 'data' array"
    );
    assert!(
        !parsed["data"].as_array().unwrap().is_empty(),
        "action data array should not be empty"
    );
}

// ─── 42. --table full shows all columns ───────────────────────────────────────

#[test]
fn test_sl_table_full() {
    let f = create_test_file();
    let path = f.path().to_str().unwrap();

    sl_cmd()
        .args(["sl", "--file", path, "--per", "session", "--table", "full"])
        .assert()
        .success()
        .stdout(predicate::str::contains("TOTAL"))
        .stdout(predicate::str::contains("Cost"));
}

// ─── 43. --order asc produces earlier rows first ──────────────────────────────

#[test]
fn test_sl_order_asc() {
    let f = create_test_file();
    let path = f.path().to_str().unwrap();

    let asc_output = sl_cmd()
        .args(["sl", "--file", path, "--per", "5h", "--order", "asc"])
        .output()
        .expect("run asc");
    let desc_output = sl_cmd()
        .args(["sl", "--file", path, "--per", "5h", "--order", "desc"])
        .output()
        .expect("run desc");

    assert!(asc_output.status.success(), "asc order should succeed");
    assert!(desc_output.status.success(), "desc order should succeed");

    let asc_str = String::from_utf8(asc_output.stdout).unwrap();
    let desc_str = String::from_utf8(desc_output.stdout).unwrap();

    // asc and desc should produce different output (rows are in different order)
    assert_ne!(
        asc_str, desc_str,
        "asc and desc should produce different output"
    );
}

// ─── 44. --tz fixed offset ────────────────────────────────────────────────────

#[test]
fn test_sl_timezone_fixed_offset() {
    let f = create_test_file();
    let path = f.path().to_str().unwrap();

    sl_cmd()
        .args(["sl", "--file", path, "--per", "day", "--tz", "+08:00"])
        .assert()
        .success()
        .stdout(predicate::str::contains("TOTAL"));
}

// ─── 45. --output tsv --per session ───────────────────────────────────────────

#[test]
fn test_sl_output_tsv_per_session() {
    let f = create_test_file();
    let path = f.path().to_str().unwrap();

    let output = sl_cmd()
        .args([
            "sl",
            "--file",
            path,
            "--per",
            "session",
            "--output",
            "tsv",
            "--filename",
            "-",
        ])
        .output()
        .expect("run command");

    assert!(
        output.status.success(),
        "sl --output tsv --per session should succeed"
    );
    let stdout = String::from_utf8(output.stdout).unwrap();
    let first_line = stdout
        .lines()
        .next()
        .expect("should have at least one line");
    assert!(
        first_line.contains('\t'),
        "TSV output should be tab-separated"
    );
    assert!(
        first_line.contains("Session"),
        "TSV header should contain 'Session'"
    );
}
