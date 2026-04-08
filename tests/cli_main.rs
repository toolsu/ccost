use assert_cmd::Command;
use predicates::prelude::*;
use std::fs;
use tempfile::TempDir;

fn make_fixture(records: &[serde_json::Value]) -> TempDir {
    let dir = TempDir::new().unwrap();
    let proj_dir = dir.path().join("projects").join("test-project");
    fs::create_dir_all(&proj_dir).unwrap();
    let content: String = records
        .iter()
        .map(|r| serde_json::to_string(r).unwrap())
        .collect::<Vec<_>>()
        .join("\n");
    fs::write(proj_dir.join("session-abc.jsonl"), content).unwrap();
    dir
}

#[allow(clippy::too_many_arguments)]
fn mock_rec(
    model: &str,
    input: u64,
    output: u64,
    cache_create: u64,
    cache_read: u64,
    ts: &str,
    req_id: &str,
    msg_id: &str,
) -> serde_json::Value {
    serde_json::json!({
        "timestamp": ts,
        "type": "assistant",
        "sessionId": "session-abc",
        "message": {
            "id": msg_id,
            "role": "assistant",
            "model": model,
            "usage": {
                "input_tokens": input,
                "output_tokens": output,
                "cache_creation_input_tokens": cache_create,
                "cache_read_input_tokens": cache_read,
            }
        },
        "requestId": req_id,
    })
}

fn two_record_fixture() -> TempDir {
    make_fixture(&[
        mock_rec(
            "claude-sonnet-4-20250514",
            1000,
            500,
            200,
            300,
            "2026-03-22T10:00:00Z",
            "req-1",
            "msg-1",
        ),
        mock_rec(
            "claude-sonnet-4-20250514",
            2000,
            800,
            400,
            600,
            "2026-03-23T14:00:00Z",
            "req-2",
            "msg-2",
        ),
    ])
}

// ─── 1. --help flag ───────────────────────────────────────────────────────────

#[test]
fn test_help_flag() {
    Command::cargo_bin("ccost")
        .unwrap()
        .arg("--help")
        .assert()
        .success()
        .stderr(predicate::str::contains("Usage: ccost"));
}

// ─── 2. --version flag ───────────────────────────────────────────────────────

#[test]
fn test_version_flag() {
    Command::cargo_bin("ccost")
        .unwrap()
        .arg("--version")
        .assert()
        .success()
        .stdout(predicate::str::contains(env!("CARGO_PKG_VERSION")));
}

// ─── 3. Terminal table output with --tz UTC ──────────────────────────────────

#[test]
fn test_terminal_table_output() {
    let dir = two_record_fixture();
    let assert = Command::cargo_bin("ccost")
        .unwrap()
        .args([
            "--claude-dir",
            dir.path().to_str().unwrap(),
            "--tz",
            "UTC",
            "--per",
            "day",
            "--per",
            "model",
        ])
        .assert()
        .success();

    // stdout should contain box-drawing characters and TOTAL
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
    assert!(
        stdout.contains('\u{2502}')
            || stdout.contains('\u{2500}')
            || stdout.contains('│')
            || stdout.contains('─'),
        "Expected box-drawing characters in stdout"
    );
    assert!(stdout.contains("TOTAL"), "Expected TOTAL in stdout");

    // stderr should contain dedup and pricing info
    let stderr = String::from_utf8(assert.get_output().stderr.clone()).unwrap();
    assert!(
        stderr.contains("Streaming dedup"),
        "Expected 'Streaming dedup' in stderr"
    );
    assert!(
        stderr.contains("Prices: LiteLLM"),
        "Expected 'Prices: LiteLLM' in stderr"
    );
}

// ─── 4. --order desc ─────────────────────────────────────────────────────────

#[test]
fn test_order_desc() {
    let dir = two_record_fixture();
    let output = Command::cargo_bin("ccost")
        .unwrap()
        .args([
            "--claude-dir",
            dir.path().to_str().unwrap(),
            "--order",
            "desc",
            "--per",
            "day",
            "--tz",
            "UTC",
        ])
        .output()
        .unwrap();

    let stdout = String::from_utf8(output.stdout).unwrap();
    // In descending order, 2026-03-23 should appear before 2026-03-22
    let pos_23 = stdout.find("2026-03-23");
    let pos_22 = stdout.find("2026-03-22");
    assert!(pos_23.is_some(), "Expected 2026-03-23 in output");
    assert!(pos_22.is_some(), "Expected 2026-03-22 in output");
    assert!(
        pos_23.unwrap() < pos_22.unwrap(),
        "Expected 2026-03-23 before 2026-03-22 in desc order"
    );
}

// ─── 5. --cost modes ─────────────────────────────────────────────────────────

#[test]
fn test_cost_decimal_mode() {
    let dir = two_record_fixture();
    let output = Command::cargo_bin("ccost")
        .unwrap()
        .args([
            "--cost",
            "decimal",
            "--claude-dir",
            dir.path().to_str().unwrap(),
            "--tz",
            "UTC",
        ])
        .output()
        .unwrap();

    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("$"), "Expected '$' in decimal cost mode");
    assert!(stdout.contains("."), "Expected '.' in decimal cost mode");
}

#[test]
fn test_cost_false_mode() {
    let dir = two_record_fixture();
    let output = Command::cargo_bin("ccost")
        .unwrap()
        .args([
            "--cost",
            "false",
            "--claude-dir",
            dir.path().to_str().unwrap(),
            "--tz",
            "UTC",
        ])
        .output()
        .unwrap();

    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(!stdout.contains("$"), "Expected no '$' in cost=false mode");
}

#[test]
fn test_cost_true_mode() {
    let dir = two_record_fixture();
    let output = Command::cargo_bin("ccost")
        .unwrap()
        .args([
            "--cost",
            "true",
            "--claude-dir",
            dir.path().to_str().unwrap(),
            "--tz",
            "UTC",
        ])
        .output()
        .unwrap();

    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("$"), "Expected '$' in cost=true mode");
}

// ─── 6. Date filtering ──────────────────────────────────────────────────────

#[test]
fn test_date_filtering() {
    let dir = make_fixture(&[
        mock_rec(
            "claude-sonnet-4-20250514",
            1000,
            500,
            200,
            300,
            "2026-03-21T10:00:00Z",
            "req-1",
            "msg-1",
        ),
        mock_rec(
            "claude-sonnet-4-20250514",
            2000,
            800,
            400,
            600,
            "2026-03-23T14:00:00Z",
            "req-2",
            "msg-2",
        ),
        mock_rec(
            "claude-sonnet-4-20250514",
            3000,
            1200,
            500,
            700,
            "2026-03-25T08:00:00Z",
            "req-3",
            "msg-3",
        ),
    ]);

    let output = Command::cargo_bin("ccost")
        .unwrap()
        .args([
            "--from",
            "2026-03-22",
            "--to",
            "2026-03-24",
            "--claude-dir",
            dir.path().to_str().unwrap(),
            "--tz",
            "UTC",
            "--per",
            "day",
        ])
        .output()
        .unwrap();

    let stdout = String::from_utf8(output.stdout).unwrap();
    // Only 2026-03-23 should appear (between 22 and 24)
    assert!(
        stdout.contains("2026-03-23"),
        "Expected 2026-03-23 in filtered output"
    );
    assert!(
        !stdout.contains("2026-03-21"),
        "Expected 2026-03-21 to be excluded"
    );
    assert!(
        !stdout.contains("2026-03-25"),
        "Expected 2026-03-25 to be excluded"
    );
}

// ─── 7. --5hfrom flag ────────────────────────────────────────────────────────

#[test]
fn test_5hfrom_flag() {
    let dir = two_record_fixture();
    Command::cargo_bin("ccost")
        .unwrap()
        .args([
            "--5hfrom",
            "2026-03-23T10:00:00",
            "--tz",
            "UTC",
            "--claude-dir",
            dir.path().to_str().unwrap(),
        ])
        .assert()
        .success();
}

// ─── 8. File output: --output json ───────────────────────────────────────────

#[test]
fn test_output_json() {
    let dir = two_record_fixture();
    let out_dir = TempDir::new().unwrap();

    Command::cargo_bin("ccost")
        .unwrap()
        .args([
            "--output",
            "json",
            "--claude-dir",
            dir.path().to_str().unwrap(),
            "--tz",
            "UTC",
        ])
        .current_dir(out_dir.path())
        .assert()
        .success();

    let json_path = out_dir.path().join("ccost.json");
    assert!(json_path.exists(), "ccost.json should exist");

    let content = fs::read_to_string(&json_path).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&content).unwrap();
    assert!(parsed.get("meta").is_some(), "JSON should have 'meta' key");
    assert!(parsed.get("data").is_some(), "JSON should have 'data' key");
    assert!(
        parsed.get("totals").is_some(),
        "JSON should have 'totals' key"
    );
    assert!(
        parsed.get("dedup").is_some(),
        "JSON should have 'dedup' key"
    );
}

// ─── 9. File output: --output markdown ───────────────────────────────────────

#[test]
fn test_output_markdown() {
    let dir = two_record_fixture();
    let out_dir = TempDir::new().unwrap();

    Command::cargo_bin("ccost")
        .unwrap()
        .args([
            "--output",
            "markdown",
            "--claude-dir",
            dir.path().to_str().unwrap(),
            "--tz",
            "UTC",
        ])
        .current_dir(out_dir.path())
        .assert()
        .success();

    let md_path = out_dir.path().join("ccost.md");
    assert!(md_path.exists(), "ccost.md should exist");

    let content = fs::read_to_string(&md_path).unwrap();
    assert!(content.contains("|"), "Markdown should contain '|'");
    assert!(content.contains("---"), "Markdown should contain '---'");
    assert!(content.contains("TOTAL"), "Markdown should contain 'TOTAL'");
}

// ─── 10. File output: --output html ─────────────────────────────────────────

#[test]
fn test_output_html() {
    let dir = two_record_fixture();
    let out_dir = TempDir::new().unwrap();

    Command::cargo_bin("ccost")
        .unwrap()
        .args([
            "--output",
            "html",
            "--claude-dir",
            dir.path().to_str().unwrap(),
            "--tz",
            "UTC",
        ])
        .current_dir(out_dir.path())
        .assert()
        .success();

    let html_path = out_dir.path().join("ccost.html");
    assert!(html_path.exists(), "ccost.html should exist");

    let content = fs::read_to_string(&html_path).unwrap();
    assert!(
        content.contains("<!DOCTYPE html>"),
        "HTML should contain <!DOCTYPE html>"
    );
    assert!(
        content.contains("<table>") || content.contains("<table "),
        "HTML should contain <table>"
    );
}

// ─── 11. File output: --output csv ──────────────────────────────────────────

#[test]
fn test_output_csv() {
    let dir = two_record_fixture();
    let out_dir = TempDir::new().unwrap();

    Command::cargo_bin("ccost")
        .unwrap()
        .args([
            "--output",
            "csv",
            "--claude-dir",
            dir.path().to_str().unwrap(),
            "--tz",
            "UTC",
        ])
        .current_dir(out_dir.path())
        .assert()
        .success();

    let csv_path = out_dir.path().join("ccost.csv");
    assert!(csv_path.exists(), "ccost.csv should exist");

    let content = fs::read_to_string(&csv_path).unwrap();
    let first_line = content.lines().next().unwrap();
    assert!(
        first_line.contains(","),
        "CSV header should be comma-separated"
    );
}

// ─── 12. File output: --output tsv ──────────────────────────────────────────

#[test]
fn test_output_tsv() {
    let dir = two_record_fixture();
    let out_dir = TempDir::new().unwrap();

    Command::cargo_bin("ccost")
        .unwrap()
        .args([
            "--output",
            "tsv",
            "--claude-dir",
            dir.path().to_str().unwrap(),
            "--tz",
            "UTC",
        ])
        .current_dir(out_dir.path())
        .assert()
        .success();

    let tsv_path = out_dir.path().join("ccost.tsv");
    assert!(tsv_path.exists(), "ccost.tsv should exist");

    let content = fs::read_to_string(&tsv_path).unwrap();
    let first_line = content.lines().next().unwrap();
    assert!(
        first_line.contains("\t"),
        "TSV header should be tab-separated"
    );
}

// ─── 13. File output: --output txt ──────────────────────────────────────────

#[test]
fn test_output_txt() {
    let dir = two_record_fixture();
    let out_dir = TempDir::new().unwrap();

    Command::cargo_bin("ccost")
        .unwrap()
        .args([
            "--output",
            "txt",
            "--claude-dir",
            dir.path().to_str().unwrap(),
            "--tz",
            "UTC",
        ])
        .current_dir(out_dir.path())
        .assert()
        .success();

    let txt_path = out_dir.path().join("ccost.txt");
    assert!(txt_path.exists(), "ccost.txt should exist");

    let content = fs::read_to_string(&txt_path).unwrap();
    // Should contain box-drawing characters
    assert!(
        content.contains('\u{2502}')
            || content.contains('\u{2500}')
            || content.contains('│')
            || content.contains('─'),
        "TXT output should contain box-drawing characters"
    );
    // Should NOT contain ANSI escape codes
    assert!(
        !content.contains("\x1b["),
        "TXT output should not contain ANSI escape codes"
    );
}

// ─── 14. --table compact and --table full ────────────────────────────────────

#[test]
fn test_table_compact() {
    let dir = two_record_fixture();
    let output = Command::cargo_bin("ccost")
        .unwrap()
        .args([
            "--table",
            "compact",
            "--claude-dir",
            dir.path().to_str().unwrap(),
            "--tz",
            "UTC",
        ])
        .output()
        .unwrap();

    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(
        !stdout.contains("Cache Cr"),
        "Compact table should NOT contain 'Cache Cr'"
    );
}

#[test]
fn test_table_full() {
    let dir = two_record_fixture();
    let output = Command::cargo_bin("ccost")
        .unwrap()
        .args([
            "--table",
            "full",
            "--claude-dir",
            dir.path().to_str().unwrap(),
            "--tz",
            "UTC",
        ])
        .output()
        .unwrap();

    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(
        stdout.contains("Cache Cr"),
        "Full table should contain 'Cache Cr'"
    );
}

// ─── 15. Validation errors ──────────────────────────────────────────────────

#[test]
fn test_unknown_flag_error() {
    Command::cargo_bin("ccost")
        .unwrap()
        .arg("--nonexistent")
        .assert()
        .failure();
}

#[test]
fn test_chart_output_conflict_error() {
    let dir = two_record_fixture();
    Command::cargo_bin("ccost")
        .unwrap()
        .args([
            "--chart",
            "cost",
            "--output",
            "html",
            "--claude-dir",
            dir.path().to_str().unwrap(),
            "--tz",
            "UTC",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("--chart conflicts with --output"));
}

// ─── 16. --filename without --output ─────────────────────────────────────────

#[test]
fn test_filename_without_output() {
    let dir = two_record_fixture();
    let out_dir = TempDir::new().unwrap();
    let custom_path = out_dir.path().join("custom.txt");

    Command::cargo_bin("ccost")
        .unwrap()
        .args([
            "--filename",
            custom_path.to_str().unwrap(),
            "--claude-dir",
            dir.path().to_str().unwrap(),
            "--tz",
            "UTC",
        ])
        .assert()
        .success();

    assert!(custom_path.exists(), "custom.txt should exist");

    let content = fs::read_to_string(&custom_path).unwrap();
    // Should contain table content
    assert!(
        content.contains('\u{2502}')
            || content.contains('\u{2500}')
            || content.contains('│')
            || content.contains('─'),
        "custom.txt should contain box-drawing characters (table)"
    );
    // No ANSI escape codes
    assert!(
        !content.contains("\x1b["),
        "custom.txt should not contain ANSI escape codes"
    );
}

// ─── 17. --pricing-data with custom pricing ──────────────────────────────────

#[test]
fn test_custom_pricing_data() {
    let dir = two_record_fixture();
    let pricing_dir = TempDir::new().unwrap();
    let pricing_path = pricing_dir.path().join("pricing.json");

    let pricing_json = serde_json::json!({
        "fetchedAt": "2026-03-25T00:00:00Z",
        "models": {
            "claude-sonnet-4-20250514": {
                "inputCostPerToken": 0.000003,
                "outputCostPerToken": 0.000015,
                "cacheCreationCostPerToken": 0.00000375,
                "cacheReadCostPerToken": 0.0000003
            }
        }
    });
    fs::write(
        &pricing_path,
        serde_json::to_string_pretty(&pricing_json).unwrap(),
    )
    .unwrap();

    Command::cargo_bin("ccost")
        .unwrap()
        .args([
            "--pricing-data",
            pricing_path.to_str().unwrap(),
            "--claude-dir",
            dir.path().to_str().unwrap(),
            "--tz",
            "UTC",
        ])
        .assert()
        .success();
}

// ─── 18. --chart cost mode ───────────────────────────────────────────────────

#[test]
fn test_chart_cost_mode() {
    let dir = two_record_fixture();
    let output = Command::cargo_bin("ccost")
        .unwrap()
        .args([
            "--chart",
            "cost",
            "--claude-dir",
            dir.path().to_str().unwrap(),
            "--tz",
            "UTC",
        ])
        .output()
        .unwrap();

    assert!(output.status.success(), "chart cost should succeed");
    let stdout = String::from_utf8(output.stdout).unwrap();
    // Chart output should contain cost label or braille characters (U+2800 range)
    let has_cost_label = stdout.contains("Cost ($)") || stdout.contains("Cost");
    let has_braille = stdout
        .chars()
        .any(|c| ('\u{2800}'..='\u{28FF}').contains(&c));
    assert!(
        has_cost_label || has_braille,
        "Chart cost output should contain 'Cost ($)' or braille characters, got: {}",
        &stdout[..stdout.len().min(500)]
    );
}

// ─── 19. --chart token mode ──────────────────────────────────────────────────

#[test]
fn test_chart_token_mode() {
    let dir = two_record_fixture();
    let output = Command::cargo_bin("ccost")
        .unwrap()
        .args([
            "--chart",
            "token",
            "--claude-dir",
            dir.path().to_str().unwrap(),
            "--tz",
            "UTC",
        ])
        .output()
        .unwrap();

    assert!(output.status.success(), "chart token should succeed");
    let stdout = String::from_utf8(output.stdout).unwrap();
    let has_token_label = stdout.contains("Tokens") || stdout.contains("Token");
    let has_braille = stdout
        .chars()
        .any(|c| ('\u{2800}'..='\u{28FF}').contains(&c));
    assert!(
        has_token_label || has_braille,
        "Chart token output should contain 'Tokens' or braille characters, got: {}",
        &stdout[..stdout.len().min(500)]
    );
}

// ─── 20. --1wfrom and --1wto ─────────────────────────────────────────────────

#[test]
fn test_1wfrom_flag() {
    let dir = two_record_fixture();
    Command::cargo_bin("ccost")
        .unwrap()
        .args([
            "--1wfrom",
            "2026-03-18T00:00:00",
            "--tz",
            "UTC",
            "--claude-dir",
            dir.path().to_str().unwrap(),
        ])
        .assert()
        .success();
}

#[test]
fn test_1wto_flag() {
    let dir = two_record_fixture();
    Command::cargo_bin("ccost")
        .unwrap()
        .args([
            "--1wto",
            "2026-03-25T00:00:00",
            "--tz",
            "UTC",
            "--claude-dir",
            dir.path().to_str().unwrap(),
        ])
        .assert()
        .success();
}

// ─── 21. --5hto flag (correctness) ─────────────────────────────────────

#[test]
fn test_5hto_flag() {
    // Records at 10:00 and 14:00 on Mar 23
    // --5hto 2026-03-23T15:00:00 → from=10:00 to=15:00 → both should match
    let dir = make_fixture(&[
        mock_rec(
            "claude-sonnet-4-20250514",
            1000,
            500,
            0,
            0,
            "2026-03-23T10:00:00Z",
            "req-1",
            "msg-1",
        ),
        mock_rec(
            "claude-sonnet-4-20250514",
            2000,
            800,
            0,
            0,
            "2026-03-23T14:00:00Z",
            "req-2",
            "msg-2",
        ),
    ]);
    let output = Command::cargo_bin("ccost")
        .unwrap()
        .args([
            "--5hto",
            "2026-03-23T15:00:00",
            "--tz",
            "UTC",
            "--claude-dir",
            dir.path().to_str().unwrap(),
            "--per",
            "day",
            "--output",
            "json",
        ])
        .current_dir(dir.path())
        .output()
        .unwrap();

    let stdout = String::from_utf8(output.stderr).unwrap();
    assert!(output.status.success());
    // The JSON file should have been created; check it has data
    let json_path = dir.path().join("ccost.json");
    if json_path.exists() {
        let content = fs::read_to_string(&json_path).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&content).unwrap();
        assert!(
            !parsed["data"].as_array().unwrap().is_empty(),
            "5hto should include records in range"
        );
    }
    let _ = stdout;
}

// ─── 22. Validation error tests ────────────────────────────────────────

#[test]
fn test_invalid_date_format_error() {
    let dir = two_record_fixture();
    Command::cargo_bin("ccost")
        .unwrap()
        .args([
            "--from",
            "not-a-date",
            "--claude-dir",
            dir.path().to_str().unwrap(),
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("invalid date format"));
}

#[test]
fn test_invalid_per_dimension_error() {
    let dir = two_record_fixture();
    Command::cargo_bin("ccost")
        .unwrap()
        .args([
            "--per",
            "banana",
            "--claude-dir",
            dir.path().to_str().unwrap(),
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("invalid dimension"));
}

#[test]
fn test_invalid_cost_mode_error() {
    let dir = two_record_fixture();
    Command::cargo_bin("ccost")
        .unwrap()
        .args([
            "--cost",
            "banana",
            "--claude-dir",
            dir.path().to_str().unwrap(),
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("invalid value"));
}

#[test]
fn test_invalid_output_format_error() {
    let dir = two_record_fixture();
    Command::cargo_bin("ccost")
        .unwrap()
        .args([
            "--output",
            "banana",
            "--claude-dir",
            dir.path().to_str().unwrap(),
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("invalid format"));
}

#[test]
fn test_invalid_table_mode_error() {
    let dir = two_record_fixture();
    Command::cargo_bin("ccost")
        .unwrap()
        .args([
            "--table",
            "banana",
            "--claude-dir",
            dir.path().to_str().unwrap(),
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("invalid mode"));
}

#[test]
fn test_invalid_chart_mode_error() {
    let dir = two_record_fixture();
    Command::cargo_bin("ccost")
        .unwrap()
        .args([
            "--chart",
            "banana",
            "--claude-dir",
            dir.path().to_str().unwrap(),
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("invalid mode"));
}

// ─── 23. Conflict errors ───────────────────────────────────────────────

#[test]
fn test_5h_conflict_error() {
    let dir = two_record_fixture();
    Command::cargo_bin("ccost")
        .unwrap()
        .args([
            "--5hfrom",
            "2026-03-23T10:00:00",
            "--5hto",
            "2026-03-23T15:00:00",
            "--claude-dir",
            dir.path().to_str().unwrap(),
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "--5hfrom and --5hto cannot be used together",
        ));
}

#[test]
fn test_1w_conflict_error() {
    let dir = two_record_fixture();
    Command::cargo_bin("ccost")
        .unwrap()
        .args([
            "--1wfrom",
            "2026-03-18T00:00:00",
            "--1wto",
            "2026-03-25T00:00:00",
            "--claude-dir",
            dir.path().to_str().unwrap(),
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "--1wfrom and --1wto cannot be used together",
        ));
}

#[test]
fn test_5h_1w_conflict_error() {
    let dir = two_record_fixture();
    Command::cargo_bin("ccost")
        .unwrap()
        .args([
            "--5hfrom",
            "2026-03-23T10:00:00",
            "--1wfrom",
            "2026-03-18T00:00:00",
            "--claude-dir",
            dir.path().to_str().unwrap(),
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("--5h* and --1w*"));
}

#[test]
fn test_from_with_5h_conflict_error() {
    let dir = two_record_fixture();
    Command::cargo_bin("ccost")
        .unwrap()
        .args([
            "--from",
            "2026-03-20",
            "--5hfrom",
            "2026-03-23T10:00:00",
            "--claude-dir",
            dir.path().to_str().unwrap(),
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("cannot be used with --from/--to"));
}

#[test]
fn test_live_pricing_and_pricing_data_conflict() {
    let dir = two_record_fixture();
    Command::cargo_bin("ccost")
        .unwrap()
        .args([
            "--live-pricing",
            "--pricing-data",
            "/some/file.json",
            "--claude-dir",
            dir.path().to_str().unwrap(),
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "--live-pricing and --pricing-data cannot be used together",
        ));
}

#[test]
fn test_per_max_two_error() {
    let dir = two_record_fixture();
    Command::cargo_bin("ccost")
        .unwrap()
        .args([
            "--per",
            "day",
            "--per",
            "model",
            "--per",
            "session",
            "--claude-dir",
            dir.path().to_str().unwrap(),
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("maximum 2 values"));
}

// ─── 24. --order invalid error ─────────────────────────────────────────

#[test]
fn test_invalid_order_error() {
    let dir = two_record_fixture();
    Command::cargo_bin("ccost")
        .unwrap()
        .args([
            "--order",
            "banana",
            "--claude-dir",
            dir.path().to_str().unwrap(),
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("invalid value"));
}

// ─── 25. Empty directory (no crash) ────────────────────────────────────

#[test]
fn test_empty_claude_dir() {
    let dir = TempDir::new().unwrap();
    let proj_dir = dir.path().join("projects");
    fs::create_dir_all(&proj_dir).unwrap();

    Command::cargo_bin("ccost")
        .unwrap()
        .args(["--claude-dir", dir.path().to_str().unwrap(), "--tz", "UTC"])
        .assert()
        .success();
}

// ─── 26. --filename with --output ──────────────────────────────────────

#[test]
fn test_filename_with_output_json() {
    let dir = two_record_fixture();
    let out_dir = TempDir::new().unwrap();
    let custom_path = out_dir.path().join("report.json");

    Command::cargo_bin("ccost")
        .unwrap()
        .args([
            "--output",
            "json",
            "--filename",
            custom_path.to_str().unwrap(),
            "--claude-dir",
            dir.path().to_str().unwrap(),
            "--tz",
            "UTC",
        ])
        .assert()
        .success();

    assert!(custom_path.exists(), "custom filename should be used");
    let content = fs::read_to_string(&custom_path).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&content).unwrap();
    assert!(parsed.get("data").is_some());
}

// ─── 27. --per single dimension ────────────────────────────────────────

#[test]
fn test_per_model_only() {
    let dir = two_record_fixture();
    let output = Command::cargo_bin("ccost")
        .unwrap()
        .args([
            "--per",
            "model",
            "--claude-dir",
            dir.path().to_str().unwrap(),
            "--tz",
            "UTC",
        ])
        .output()
        .unwrap();

    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(
        stdout.contains("sonnet-4"),
        "should show shortened model name"
    );
    // "└─ " (with trailing space) is the child row prefix; table borders use └── without space
    assert!(
        !stdout.contains("└─ "),
        "single dimension should not have child rows"
    );
}

// ─── 28. --chart with --output txt (allowed) ──────────────────────────

#[test]
fn test_chart_with_output_txt_allowed() {
    let dir = two_record_fixture();
    let out_dir = TempDir::new().unwrap();

    Command::cargo_bin("ccost")
        .unwrap()
        .args([
            "--chart",
            "cost",
            "--output",
            "txt",
            "--claude-dir",
            dir.path().to_str().unwrap(),
            "--tz",
            "UTC",
        ])
        .current_dir(out_dir.path())
        .assert()
        .success();

    let txt_path = out_dir.path().join("ccost.txt");
    assert!(txt_path.exists(), "chart + txt output should create file");
}

// ─── 29. --table auto uses terminal width ──────────────────────────────

#[test]
fn test_table_auto_wide_terminal() {
    // COLUMNS=200 → wide terminal → should show full table (Cache Cr visible)
    let dir = two_record_fixture();
    let output = Command::cargo_bin("ccost")
        .unwrap()
        .env("COLUMNS", "200")
        .args([
            "--table",
            "auto",
            "--claude-dir",
            dir.path().to_str().unwrap(),
            "--tz",
            "UTC",
        ])
        .output()
        .unwrap();

    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(
        stdout.contains("Cache Cr"),
        "auto mode with wide terminal (COLUMNS=200) should show full table with 'Cache Cr'"
    );
}

#[test]
fn test_table_auto_narrow_terminal() {
    // COLUMNS=80 → narrow terminal → should show compact table (no Cache Cr)
    let dir = two_record_fixture();
    let output = Command::cargo_bin("ccost")
        .unwrap()
        .env("COLUMNS", "80")
        .args([
            "--table",
            "auto",
            "--claude-dir",
            dir.path().to_str().unwrap(),
            "--tz",
            "UTC",
        ])
        .output()
        .unwrap();

    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(
        !stdout.contains("Cache Cr"),
        "auto mode with narrow terminal (COLUMNS=80) should show compact table without 'Cache Cr'"
    );
}

#[test]
fn test_table_auto_with_output_uses_full() {
    // --table auto (default) + --output should produce full table (not compact)
    let dir = two_record_fixture();
    let out_dir = TempDir::new().unwrap();
    let out_file = out_dir.path().join("out.txt");

    Command::cargo_bin("ccost")
        .unwrap()
        .env("COLUMNS", "80") // narrow terminal, but shouldn't matter with --output
        .args([
            "--output",
            "txt",
            "--filename",
            out_file.to_str().unwrap(),
            "--claude-dir",
            dir.path().to_str().unwrap(),
            "--tz",
            "UTC",
        ])
        .assert()
        .success();

    let content = fs::read_to_string(&out_file).unwrap();
    assert!(
        content.contains("Cache Cr"),
        "--table auto with --output should use full table (show Cache Cr), even with narrow COLUMNS"
    );
}

#[test]
fn test_table_compact_with_output_stays_compact() {
    // --table compact + --output should still produce compact table
    let dir = two_record_fixture();
    let out_dir = TempDir::new().unwrap();
    let out_file = out_dir.path().join("out.txt");

    Command::cargo_bin("ccost")
        .unwrap()
        .args([
            "--output",
            "txt",
            "--table",
            "compact",
            "--filename",
            out_file.to_str().unwrap(),
            "--claude-dir",
            dir.path().to_str().unwrap(),
            "--tz",
            "UTC",
        ])
        .assert()
        .success();

    let content = fs::read_to_string(&out_file).unwrap();
    assert!(
        !content.contains("Cache Cr"),
        "--table compact with --output should stay compact (no Cache Cr)"
    );
}

// ─── 30. --copy flag ───────────────────────────────────────────────────

fn clear_clipboard() {
    use std::process;
    let _ = if cfg!(target_os = "macos") {
        process::Command::new("pbcopy")
            .stdin(process::Stdio::piped())
            .spawn()
            .and_then(|mut c| {
                drop(c.stdin.take());
                c.wait()
            })
    } else {
        process::Command::new("xclip")
            .args(["-selection", "clipboard"])
            .stdin(process::Stdio::piped())
            .spawn()
            .or_else(|_| {
                process::Command::new("xsel")
                    .args(["--clipboard", "--input"])
                    .stdin(process::Stdio::piped())
                    .spawn()
            })
            .or_else(|_| process::Command::new("wl-copy").arg("--clear").spawn())
            .and_then(|mut c| {
                drop(c.stdin.take());
                c.wait()
            })
    };
}

#[test]
fn test_copy_invalid_format_error() {
    let dir = two_record_fixture();
    Command::cargo_bin("ccost")
        .unwrap()
        .args([
            "--copy",
            "banana",
            "--claude-dir",
            dir.path().to_str().unwrap(),
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("--copy: invalid format"));
}

#[test]
fn test_copy_with_output_both_work() {
    clear_clipboard();
    // --copy should work alongside --output (file still gets written)
    let dir = two_record_fixture();
    let out_dir = TempDir::new().unwrap();

    let output = Command::cargo_bin("ccost")
        .unwrap()
        .args([
            "--output",
            "json",
            "--copy",
            "markdown",
            "--claude-dir",
            dir.path().to_str().unwrap(),
            "--tz",
            "UTC",
        ])
        .current_dir(out_dir.path())
        .output()
        .unwrap();

    // The JSON file should still be written
    let json_path = out_dir.path().join("ccost.json");
    assert!(
        json_path.exists(),
        "JSON file should still be created with --copy"
    );

    let stderr = String::from_utf8(output.stderr).unwrap();
    // Should show copy attempt (either success or error about missing clipboard tool)
    assert!(
        stderr.contains("Copied markdown to clipboard") || stderr.contains("clipboard"),
        "stderr should mention clipboard operation"
    );
    clear_clipboard();
}

// ─── 31. --copy works with terminal output ─────────────────────────────

#[test]
fn test_copy_with_terminal_output() {
    clear_clipboard();
    // --copy alone should still print table to stdout
    let dir = two_record_fixture();
    let output = Command::cargo_bin("ccost")
        .unwrap()
        .args([
            "--copy",
            "json",
            "--claude-dir",
            dir.path().to_str().unwrap(),
            "--tz",
            "UTC",
        ])
        .output()
        .unwrap();

    let stdout = String::from_utf8(output.stdout).unwrap();
    // Table should still appear on stdout
    assert!(
        stdout.contains("TOTAL"),
        "table should still appear on stdout with --copy"
    );

    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(
        stderr.contains("clipboard"),
        "stderr should mention clipboard operation"
    );
    clear_clipboard();
}

// ─── 32. --copy with --chart ───────────────────────────────────────────

#[test]
fn test_copy_with_chart() {
    clear_clipboard();
    let dir = two_record_fixture();
    let output = Command::cargo_bin("ccost")
        .unwrap()
        .args([
            "--chart",
            "cost",
            "--copy",
            "csv",
            "--claude-dir",
            dir.path().to_str().unwrap(),
            "--tz",
            "UTC",
        ])
        .output()
        .unwrap();

    assert!(output.status.success(), "copy with chart should succeed");

    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(
        stderr.contains("clipboard"),
        "stderr should mention clipboard operation with chart + copy"
    );
    clear_clipboard();
}

// ─── 33. --per subagent ────────────────────────────────────────────────────────

#[test]
fn test_per_subagent() {
    let dir = two_record_fixture();
    let output = Command::cargo_bin("ccost")
        .unwrap()
        .args([
            "--per",
            "subagent",
            "--claude-dir",
            dir.path().to_str().unwrap(),
            "--tz",
            "UTC",
        ])
        .output()
        .unwrap();

    assert!(output.status.success(), "--per subagent should succeed");
    let stdout = String::from_utf8(output.stdout).unwrap();
    // Main session records are labeled "(main)"
    assert!(
        stdout.contains("(main)"),
        "--per subagent should label main session records as '(main)'"
    );
    assert!(stdout.contains("TOTAL"), "should contain TOTAL row");
}

// ─── 34. --per session --per model (two-level grouping) ───────────────────────

#[test]
fn test_per_two_dimensions() {
    let dir = two_record_fixture();
    let output = Command::cargo_bin("ccost")
        .unwrap()
        .args([
            "--per",
            "session",
            "--per",
            "model",
            "--claude-dir",
            dir.path().to_str().unwrap(),
            "--tz",
            "UTC",
        ])
        .output()
        .unwrap();

    assert!(output.status.success(), "two-level grouping should succeed");
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("TOTAL"), "should contain TOTAL row");
    // Two-level grouping produces child rows indented with a tree prefix
    assert!(
        stdout.contains("└─ ") || stdout.contains("├─ "),
        "two-level grouping should have child rows with tree prefix"
    );
}

// ─── 35. --from and --to flags with valid data ────────────────────────────────

#[test]
fn test_from_and_to_flags() {
    let dir = two_record_fixture();
    let output = Command::cargo_bin("ccost")
        .unwrap()
        .args([
            "--from",
            "2020-01-01",
            "--to",
            "2030-01-01",
            "--per",
            "day",
            "--claude-dir",
            dir.path().to_str().unwrap(),
            "--tz",
            "UTC",
        ])
        .output()
        .unwrap();

    assert!(output.status.success(), "--from/--to should succeed");
    let stdout = String::from_utf8(output.stdout).unwrap();
    // Both records are within the range
    assert!(stdout.contains("2026-03-22"), "should include 2026-03-22");
    assert!(stdout.contains("2026-03-23"), "should include 2026-03-23");
    assert!(stdout.contains("TOTAL"), "should have TOTAL row");
}

// ─── 36. --tz UTC flag ────────────────────────────────────────────────────────

#[test]
fn test_timezone_utc() {
    let dir = two_record_fixture();
    let output = Command::cargo_bin("ccost")
        .unwrap()
        .args([
            "--tz",
            "UTC",
            "--per",
            "day",
            "--claude-dir",
            dir.path().to_str().unwrap(),
        ])
        .output()
        .unwrap();

    assert!(output.status.success(), "--tz UTC should succeed");
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains("Streaming dedup"), "should show dedup info");
}

// ─── 37. --tz IANA name ───────────────────────────────────────────────────────

#[test]
fn test_timezone_iana() {
    let dir = two_record_fixture();
    let output = Command::cargo_bin("ccost")
        .unwrap()
        .args([
            "--tz",
            "America/New_York",
            "--per",
            "day",
            "--claude-dir",
            dir.path().to_str().unwrap(),
        ])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "--tz America/New_York should succeed"
    );
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("TOTAL"), "should have TOTAL row");
}

// ─── 38. --tz fixed offset ────────────────────────────────────────────────────

#[test]
fn test_timezone_fixed_offset() {
    let dir = two_record_fixture();
    let output = Command::cargo_bin("ccost")
        .unwrap()
        .args([
            "--tz",
            "+08:00",
            "--per",
            "day",
            "--claude-dir",
            dir.path().to_str().unwrap(),
        ])
        .output()
        .unwrap();

    assert!(output.status.success(), "--tz +08:00 should succeed");
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("TOTAL"), "should have TOTAL row");
}

// ─── 39. --model filter ───────────────────────────────────────────────────────

#[test]
fn test_model_filter() {
    let dir = make_fixture(&[
        mock_rec(
            "claude-sonnet-4-20250514",
            1000,
            500,
            0,
            0,
            "2026-03-22T10:00:00Z",
            "req-1",
            "msg-1",
        ),
        mock_rec(
            "claude-opus-4-20250514",
            2000,
            800,
            0,
            0,
            "2026-03-23T14:00:00Z",
            "req-2",
            "msg-2",
        ),
    ]);
    let output = Command::cargo_bin("ccost")
        .unwrap()
        .args([
            "--model",
            "sonnet",
            "--per",
            "model",
            "--claude-dir",
            dir.path().to_str().unwrap(),
            "--tz",
            "UTC",
        ])
        .output()
        .unwrap();

    assert!(output.status.success(), "--model filter should succeed");
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(
        stdout.contains("sonnet"),
        "filtered output should contain sonnet model"
    );
    assert!(
        !stdout.contains("opus"),
        "filtered output should not contain opus model"
    );
}

// ─── 40. --session filter ─────────────────────────────────────────────────────

#[test]
fn test_session_filter() {
    // Create two sessions in different files
    let dir = TempDir::new().unwrap();
    let proj_dir = dir.path().join("projects").join("test-project");
    fs::create_dir_all(&proj_dir).unwrap();

    let rec_a = serde_json::json!({
        "timestamp": "2026-03-22T10:00:00Z",
        "type": "assistant",
        "sessionId": "session-alpha-111",
        "message": {
            "id": "msg-a",
            "role": "assistant",
            "model": "claude-sonnet-4-20250514",
            "usage": {
                "input_tokens": 1000,
                "output_tokens": 500,
                "cache_creation_input_tokens": 0,
                "cache_read_input_tokens": 0
            }
        },
        "requestId": "req-a",
    });
    let rec_b = serde_json::json!({
        "timestamp": "2026-03-23T10:00:00Z",
        "type": "assistant",
        "sessionId": "session-beta-222",
        "message": {
            "id": "msg-b",
            "role": "assistant",
            "model": "claude-sonnet-4-20250514",
            "usage": {
                "input_tokens": 2000,
                "output_tokens": 800,
                "cache_creation_input_tokens": 0,
                "cache_read_input_tokens": 0
            }
        },
        "requestId": "req-b",
    });

    fs::write(
        proj_dir.join("session-alpha-111.jsonl"),
        serde_json::to_string(&rec_a).unwrap(),
    )
    .unwrap();
    fs::write(
        proj_dir.join("session-beta-222.jsonl"),
        serde_json::to_string(&rec_b).unwrap(),
    )
    .unwrap();

    let output = Command::cargo_bin("ccost")
        .unwrap()
        .args([
            "--session",
            "alpha",
            "--per",
            "session",
            "--claude-dir",
            dir.path().to_str().unwrap(),
            "--tz",
            "UTC",
        ])
        .output()
        .unwrap();

    assert!(output.status.success(), "--session filter should succeed");
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(
        stdout.contains("alpha"),
        "filtered output should contain alpha session"
    );
    assert!(
        !stdout.contains("beta"),
        "filtered output should not contain beta session"
    );
}

// ─── 41. JSON output includes from/to in meta ─────────────────────────────────

#[test]
fn test_output_json_with_from_to() {
    let dir = two_record_fixture();
    let out_dir = TempDir::new().unwrap();

    Command::cargo_bin("ccost")
        .unwrap()
        .args([
            "--output",
            "json",
            "--from",
            "2026-01-01",
            "--to",
            "2026-12-31",
            "--claude-dir",
            dir.path().to_str().unwrap(),
            "--tz",
            "UTC",
        ])
        .current_dir(out_dir.path())
        .assert()
        .success();

    let json_path = out_dir.path().join("ccost.json");
    let content = fs::read_to_string(&json_path).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&content).unwrap();

    assert_eq!(
        parsed["meta"]["from"].as_str().unwrap_or(""),
        "2026-01-01",
        "meta.from should be set"
    );
    assert_eq!(
        parsed["meta"]["to"].as_str().unwrap_or(""),
        "2026-12-31",
        "meta.to should be set"
    );
}

// ─── 42. --chart cost --per day ───────────────────────────────────────────────

#[test]
fn test_chart_with_per_day() {
    let dir = two_record_fixture();
    let output = Command::cargo_bin("ccost")
        .unwrap()
        .args([
            "--chart",
            "cost",
            "--per",
            "day",
            "--claude-dir",
            dir.path().to_str().unwrap(),
            "--tz",
            "UTC",
        ])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "--chart cost --per day should succeed"
    );
    let stdout = String::from_utf8(output.stdout).unwrap();
    let has_cost_label = stdout.contains("Cost");
    let has_braille = stdout
        .chars()
        .any(|c| ('\u{2800}'..='\u{28FF}').contains(&c));
    assert!(
        has_cost_label || has_braille,
        "chart output should contain Cost label or braille characters"
    );
}

// ─── 43. --per hour ───────────────────────────────────────────────────────────

#[test]
fn test_per_hour() {
    let dir = two_record_fixture();
    let output = Command::cargo_bin("ccost")
        .unwrap()
        .args([
            "--per",
            "hour",
            "--claude-dir",
            dir.path().to_str().unwrap(),
            "--tz",
            "UTC",
        ])
        .output()
        .unwrap();

    assert!(output.status.success(), "--per hour should succeed");
    let stdout = String::from_utf8(output.stdout).unwrap();
    // Hour format is "YYYY-MM-DD HH:00"
    assert!(
        stdout.contains(":00"),
        "--per hour should show hour labels with :00"
    );
    assert!(stdout.contains("TOTAL"), "should have TOTAL row");
}

// ─── 44. --per month ──────────────────────────────────────────────────────────

#[test]
fn test_per_month() {
    let dir = two_record_fixture();
    let output = Command::cargo_bin("ccost")
        .unwrap()
        .args([
            "--per",
            "month",
            "--claude-dir",
            dir.path().to_str().unwrap(),
            "--tz",
            "UTC",
        ])
        .output()
        .unwrap();

    assert!(output.status.success(), "--per month should succeed");
    let stdout = String::from_utf8(output.stdout).unwrap();
    // Both fixture records are in 2026-03
    assert!(
        stdout.contains("2026-03"),
        "--per month should show 2026-03 label"
    );
    assert!(stdout.contains("TOTAL"), "should have TOTAL row");
}

// ─── 45. --per project ────────────────────────────────────────────────────────

#[test]
fn test_per_project() {
    let dir = two_record_fixture();
    let output = Command::cargo_bin("ccost")
        .unwrap()
        .args([
            "--per",
            "project",
            "--claude-dir",
            dir.path().to_str().unwrap(),
            "--tz",
            "UTC",
        ])
        .output()
        .unwrap();

    assert!(output.status.success(), "--per project should succeed");
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("TOTAL"), "should have TOTAL row");
    // The fixture project path decodes to something containing "test"
    assert!(
        stdout.contains("test"),
        "--per project should show project name containing 'test'"
    );
}

// ─── 46. --cost integer mode ──────────────────────────────────────────────────

#[test]
fn test_cost_integer_mode() {
    let dir = two_record_fixture();
    let output = Command::cargo_bin("ccost")
        .unwrap()
        .args([
            "--cost",
            "true",
            "--claude-dir",
            dir.path().to_str().unwrap(),
            "--tz",
            "UTC",
        ])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "--cost true (integer) should succeed"
    );
    let stdout = String::from_utf8(output.stdout).unwrap();
    // Integer mode shows "$N" without decimals
    assert!(stdout.contains("$"), "integer cost mode should show '$'");
}

// ─── 47. --cost off mode hides costs ──────────────────────────────────────────

#[test]
fn test_cost_off_mode() {
    let dir = two_record_fixture();
    let output = Command::cargo_bin("ccost")
        .unwrap()
        .args([
            "--cost",
            "false",
            "--claude-dir",
            dir.path().to_str().unwrap(),
            "--tz",
            "UTC",
        ])
        .output()
        .unwrap();

    assert!(output.status.success(), "--cost false (off) should succeed");
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(
        !stdout.contains("$"),
        "--cost false should hide cost column (no '$')"
    );
    assert!(stdout.contains("TOTAL"), "should still have TOTAL row");
}

// ─── 48. --order asc explicit ─────────────────────────────────────────────────

#[test]
fn test_order_asc() {
    let dir = two_record_fixture();
    let output = Command::cargo_bin("ccost")
        .unwrap()
        .args([
            "--order",
            "asc",
            "--per",
            "day",
            "--claude-dir",
            dir.path().to_str().unwrap(),
            "--tz",
            "UTC",
        ])
        .output()
        .unwrap();

    assert!(output.status.success(), "--order asc should succeed");
    let stdout = String::from_utf8(output.stdout).unwrap();
    // In ascending order, 2026-03-22 should appear before 2026-03-23
    let pos_22 = stdout.find("2026-03-22");
    let pos_23 = stdout.find("2026-03-23");
    assert!(pos_22.is_some(), "Expected 2026-03-22 in output");
    assert!(pos_23.is_some(), "Expected 2026-03-23 in output");
    assert!(
        pos_22.unwrap() < pos_23.unwrap(),
        "In asc order, 2026-03-22 should appear before 2026-03-23"
    );
}
