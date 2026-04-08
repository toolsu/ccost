use std::fs;
use std::path::Path;

use tempfile::TempDir;

use ccost::formatters::json::{format_json, JsonMeta};
use ccost::formatters::table::{format_table, TableOptions};
use ccost::*;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Create a temp dir with JSONL fixture under `projects/test-project/session-abc.jsonl`.
fn make_fixture(records: &[serde_json::Value]) -> TempDir {
    make_fixture_in_project(records, "test-project", "session-abc.jsonl")
}

/// Create a temp dir with JSONL fixture under an arbitrary project name and session file.
fn make_fixture_in_project(
    records: &[serde_json::Value],
    project: &str,
    session_file: &str,
) -> TempDir {
    let dir = TempDir::new().unwrap();
    let proj_dir = dir.path().join("projects").join(project);
    fs::create_dir_all(&proj_dir).unwrap();
    let content: String = records
        .iter()
        .map(|r| serde_json::to_string(r).unwrap())
        .collect::<Vec<_>>()
        .join("\n");
    fs::write(proj_dir.join(session_file), content).unwrap();
    dir
}

/// Build a single mock JSONL record.
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

fn default_load_opts(dir: &Path) -> LoadOptions {
    LoadOptions {
        claude_dir: Some(dir.to_string_lossy().to_string()),
        tz: Some("UTC".to_string()),
        ..Default::default()
    }
}

fn default_group_opts() -> GroupOptions {
    GroupOptions {
        order: SortOrder::Asc,
        tz: Some("UTC".to_string()),
    }
}

fn approx_eq(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-10
}

// ---------------------------------------------------------------------------
// 1. Full pipeline end-to-end
// ---------------------------------------------------------------------------

#[test]
fn test_full_pipeline_end_to_end() {
    let records = vec![
        mock_rec(
            "claude-opus-4-6",
            100,
            50,
            0,
            0,
            "2026-03-23T10:00:00Z",
            "req_1",
            "msg_1",
        ),
        mock_rec(
            "claude-3-5-haiku-20241022",
            200,
            80,
            10,
            20,
            "2026-03-23T11:00:00Z",
            "req_2",
            "msg_2",
        ),
    ];
    let dir = make_fixture(&records);
    let opts = default_load_opts(dir.path());

    // Load
    let result = load_records(&opts);
    assert_eq!(result.records.len(), 2, "should load 2 records");
    assert_eq!(result.dedup.before, 2);
    assert_eq!(result.dedup.after, 2, "no duplicates so dedup 2 -> 2");

    // Price
    let pricing = load_pricing();
    let priced = calculate_cost(&result.records, Some(&pricing));
    assert_eq!(priced.len(), 2);
    for p in &priced {
        // Every priced record should have a total_cost field (may be 0 for unknown models)
        assert!(p.total_cost >= 0.0);
        assert!(p.input_cost >= 0.0);
        assert!(p.output_cost >= 0.0);
    }

    // Group by day + model
    let dims = vec![GroupDimension::Day, GroupDimension::Model];
    let group_opts = default_group_opts();
    let grouped = group_records(&priced, &dims, Some(&group_opts));

    // All records are on the same day so we expect 1 day group
    assert_eq!(grouped.data.len(), 1, "should have 1 day group");
    let day_group = &grouped.data[0];
    assert_eq!(day_group.label, "2026-03-23");

    // That day group should have 2 model children
    let children = day_group.children.as_ref().unwrap();
    assert_eq!(children.len(), 2, "day group should have 2 model children");

    // Parent totals should equal sum of children
    let children_input_sum: u64 = children.iter().map(|c| c.input_tokens).sum();
    let children_output_sum: u64 = children.iter().map(|c| c.output_tokens).sum();
    let children_cost_sum: f64 = children.iter().map(|c| c.total_cost).sum();

    assert_eq!(day_group.input_tokens, children_input_sum);
    assert_eq!(day_group.output_tokens, children_output_sum);
    assert!(approx_eq(day_group.total_cost, children_cost_sum));

    // Grand totals should equal sum of all priced records
    let expected_input: u64 = priced.iter().map(|p| p.input_tokens).sum();
    let expected_output: u64 = priced.iter().map(|p| p.output_tokens).sum();
    assert_eq!(grouped.totals.input_tokens, expected_input);
    assert_eq!(grouped.totals.output_tokens, expected_output);
}

// ---------------------------------------------------------------------------
// 2. Streaming deduplication
// ---------------------------------------------------------------------------

#[test]
fn test_streaming_deduplication() {
    // 3 entries with same msg_id + req_id but different output_tokens (10, 30, 50)
    let records = vec![
        mock_rec(
            "claude-opus-4-6",
            100,
            10,
            0,
            0,
            "2026-03-23T10:00:00Z",
            "req_dup",
            "msg_dup",
        ),
        mock_rec(
            "claude-opus-4-6",
            100,
            30,
            0,
            0,
            "2026-03-23T10:00:00Z",
            "req_dup",
            "msg_dup",
        ),
        mock_rec(
            "claude-opus-4-6",
            100,
            50,
            0,
            0,
            "2026-03-23T10:00:00Z",
            "req_dup",
            "msg_dup",
        ),
    ];
    let dir = make_fixture(&records);
    let opts = default_load_opts(dir.path());

    let result = load_records(&opts);

    // Dedup stats: 3 -> 1
    assert_eq!(result.dedup.before, 3);
    assert_eq!(result.dedup.after, 1);

    // Surviving record should have output_tokens = 50 (highest)
    assert_eq!(result.records.len(), 1);
    assert_eq!(result.records[0].output_tokens, 50);
}

// ---------------------------------------------------------------------------
// 3. Two-level grouping
// ---------------------------------------------------------------------------

#[test]
fn test_two_level_grouping() {
    // 4 records across 2 days, different models
    let records = vec![
        mock_rec(
            "claude-opus-4-6",
            100,
            50,
            0,
            0,
            "2026-03-22T10:00:00Z",
            "req_1",
            "msg_1",
        ),
        mock_rec(
            "claude-3-5-haiku-20241022",
            200,
            80,
            0,
            0,
            "2026-03-22T11:00:00Z",
            "req_2",
            "msg_2",
        ),
        mock_rec(
            "claude-opus-4-6",
            150,
            60,
            0,
            0,
            "2026-03-23T10:00:00Z",
            "req_3",
            "msg_3",
        ),
        mock_rec(
            "claude-3-5-haiku-20241022",
            250,
            90,
            0,
            0,
            "2026-03-23T11:00:00Z",
            "req_4",
            "msg_4",
        ),
    ];
    let dir = make_fixture(&records);
    let opts = default_load_opts(dir.path());
    let result = load_records(&opts);
    assert_eq!(result.records.len(), 4);

    let pricing = load_pricing();
    let priced = calculate_cost(&result.records, Some(&pricing));
    let dims = vec![GroupDimension::Day, GroupDimension::Model];
    let group_opts = default_group_opts();
    let grouped = group_records(&priced, &dims, Some(&group_opts));

    // 2 day groups (asc order)
    assert_eq!(grouped.data.len(), 2);
    assert_eq!(grouped.data[0].label, "2026-03-22");
    assert_eq!(grouped.data[1].label, "2026-03-23");

    // Each day has 2 model children
    for day_group in &grouped.data {
        let children = day_group.children.as_ref().unwrap();
        assert_eq!(children.len(), 2, "each day should have 2 model children");

        // Parent totals = sum of children
        let children_input: u64 = children.iter().map(|c| c.input_tokens).sum();
        let children_output: u64 = children.iter().map(|c| c.output_tokens).sum();
        let children_cost: f64 = children.iter().map(|c| c.total_cost).sum();

        assert_eq!(day_group.input_tokens, children_input);
        assert_eq!(day_group.output_tokens, children_output);
        assert!(
            approx_eq(day_group.total_cost, children_cost),
            "parent totalCost ({}) should equal sum of children totalCost ({})",
            day_group.total_cost,
            children_cost,
        );
    }

    // Grand totals
    let total_input: u64 = priced.iter().map(|p| p.input_tokens).sum();
    let total_output: u64 = priced.iter().map(|p| p.output_tokens).sum();
    assert_eq!(grouped.totals.input_tokens, total_input);
    assert_eq!(grouped.totals.output_tokens, total_output);
}

// ---------------------------------------------------------------------------
// 4. Date range filter
// ---------------------------------------------------------------------------

#[test]
fn test_date_range_filter() {
    let records = vec![
        mock_rec(
            "claude-opus-4-6",
            100,
            50,
            0,
            0,
            "2026-03-20T10:00:00Z",
            "req_1",
            "msg_1",
        ),
        mock_rec(
            "claude-opus-4-6",
            200,
            80,
            0,
            0,
            "2026-03-23T10:00:00Z",
            "req_2",
            "msg_2",
        ),
        mock_rec(
            "claude-opus-4-6",
            300,
            120,
            0,
            0,
            "2026-03-26T10:00:00Z",
            "req_3",
            "msg_3",
        ),
    ];
    let dir = make_fixture(&records);

    let opts = LoadOptions {
        claude_dir: Some(dir.path().to_string_lossy().to_string()),
        tz: Some("UTC".to_string()),
        from: Some("2026-03-22".to_string()),
        to: Some("2026-03-24".to_string()),
        ..Default::default()
    };

    let result = load_records(&opts);

    // Only the Mar 23 record should pass
    assert_eq!(result.records.len(), 1);
    assert_eq!(result.records[0].input_tokens, 200);
    assert_eq!(result.records[0].output_tokens, 80);
}

// ---------------------------------------------------------------------------
// 5. Model filter
// ---------------------------------------------------------------------------

#[test]
fn test_model_filter() {
    let records = vec![
        mock_rec(
            "claude-opus-4-6",
            100,
            50,
            0,
            0,
            "2026-03-23T10:00:00Z",
            "req_1",
            "msg_1",
        ),
        mock_rec(
            "claude-3-5-haiku-20241022",
            200,
            80,
            0,
            0,
            "2026-03-23T11:00:00Z",
            "req_2",
            "msg_2",
        ),
        mock_rec(
            "claude-3-5-sonnet-20241022",
            300,
            120,
            0,
            0,
            "2026-03-23T12:00:00Z",
            "req_3",
            "msg_3",
        ),
    ];
    let dir = make_fixture(&records);

    let opts = LoadOptions {
        claude_dir: Some(dir.path().to_string_lossy().to_string()),
        tz: Some("UTC".to_string()),
        model: Some("opus".to_string()),
        ..Default::default()
    };

    let result = load_records(&opts);

    // Only the opus record should pass (case-insensitive substring)
    assert_eq!(result.records.len(), 1);
    assert!(result.records[0].model.contains("opus"));
}

// ---------------------------------------------------------------------------
// 6. Sort order
// ---------------------------------------------------------------------------

#[test]
fn test_sort_order_asc() {
    let records = vec![
        mock_rec(
            "claude-opus-4-6",
            100,
            50,
            0,
            0,
            "2026-03-22T10:00:00Z",
            "req_1",
            "msg_1",
        ),
        mock_rec(
            "claude-opus-4-6",
            200,
            80,
            0,
            0,
            "2026-03-24T10:00:00Z",
            "req_2",
            "msg_2",
        ),
    ];
    let dir = make_fixture(&records);
    let opts = default_load_opts(dir.path());
    let result = load_records(&opts);
    let pricing = load_pricing();
    let priced = calculate_cost(&result.records, Some(&pricing));

    let dims = vec![GroupDimension::Day];
    let group_opts_asc = GroupOptions {
        order: SortOrder::Asc,
        tz: Some("UTC".to_string()),
    };
    let grouped = group_records(&priced, &dims, Some(&group_opts_asc));

    assert_eq!(grouped.data.len(), 2);
    assert_eq!(
        grouped.data[0].label, "2026-03-22",
        "asc: first should be earlier date"
    );
    assert_eq!(grouped.data[1].label, "2026-03-24");
}

#[test]
fn test_sort_order_desc() {
    let records = vec![
        mock_rec(
            "claude-opus-4-6",
            100,
            50,
            0,
            0,
            "2026-03-22T10:00:00Z",
            "req_1",
            "msg_1",
        ),
        mock_rec(
            "claude-opus-4-6",
            200,
            80,
            0,
            0,
            "2026-03-24T10:00:00Z",
            "req_2",
            "msg_2",
        ),
    ];
    let dir = make_fixture(&records);
    let opts = default_load_opts(dir.path());
    let result = load_records(&opts);
    let pricing = load_pricing();
    let priced = calculate_cost(&result.records, Some(&pricing));

    let dims = vec![GroupDimension::Day];
    let group_opts_desc = GroupOptions {
        order: SortOrder::Desc,
        tz: Some("UTC".to_string()),
    };
    let grouped = group_records(&priced, &dims, Some(&group_opts_desc));

    assert_eq!(grouped.data.len(), 2);
    assert_eq!(
        grouped.data[0].label, "2026-03-24",
        "desc: first should be later date"
    );
    assert_eq!(grouped.data[1].label, "2026-03-22");
}

// ---------------------------------------------------------------------------
// 7. Table formatter output
// ---------------------------------------------------------------------------

#[test]
fn test_table_formatter_output() {
    let records = vec![
        mock_rec(
            "claude-opus-4-6",
            100,
            50,
            10,
            20,
            "2026-03-23T10:00:00Z",
            "req_1",
            "msg_1",
        ),
        mock_rec(
            "claude-3-5-haiku-20241022",
            200,
            80,
            30,
            40,
            "2026-03-23T11:00:00Z",
            "req_2",
            "msg_2",
        ),
    ];
    let dir = make_fixture(&records);
    let opts = default_load_opts(dir.path());
    let result = load_records(&opts);
    let pricing = load_pricing();
    let priced = calculate_cost(&result.records, Some(&pricing));

    let dims = vec![GroupDimension::Day, GroupDimension::Model];
    let group_opts = default_group_opts();
    let grouped = group_records(&priced, &dims, Some(&group_opts));

    let table_opts = TableOptions {
        dimension_label: "Date".to_string(),
        price_mode: PriceMode::Decimal,
        compact: false,
        color: Some(false),
    };
    let output = format_table(&grouped.data, &grouped.totals, &table_opts);

    // Box-drawing characters
    assert!(
        output.contains('\u{250C}'),
        "should contain top-left corner"
    );
    assert!(
        output.contains('\u{2500}'),
        "should contain horizontal line"
    );
    assert!(output.contains('\u{2502}'), "should contain vertical line");
    assert!(
        output.contains('\u{2518}'),
        "should contain bottom-right corner"
    );

    // Child prefix
    assert!(
        output.contains("\u{2514}\u{2500}"),
        "should contain child prefix └─"
    );

    // TOTAL row
    assert!(output.contains("TOTAL"), "should contain TOTAL row");
}

// ---------------------------------------------------------------------------
// 8. JSON formatter output
// ---------------------------------------------------------------------------

#[test]
fn test_json_formatter_output() {
    let records = vec![mock_rec(
        "claude-opus-4-6",
        100,
        50,
        0,
        0,
        "2026-03-23T10:00:00Z",
        "req_1",
        "msg_1",
    )];
    let dir = make_fixture(&records);
    let opts = default_load_opts(dir.path());
    let result = load_records(&opts);
    let pricing = load_pricing();
    let priced = calculate_cost(&result.records, Some(&pricing));

    let dims = vec![GroupDimension::Day, GroupDimension::Model];
    let group_opts = default_group_opts();
    let grouped = group_records(&priced, &dims, Some(&group_opts));

    let meta = JsonMeta {
        dimensions: vec!["day".to_string(), "model".to_string()],
        from: None,
        to: None,
        tz: Some("UTC".to_string()),
        project: None,
        model: None,
        session: None,
        order: "asc".to_string(),
        earliest: result.meta.earliest.map(|e| e.to_rfc3339()),
        latest: result.meta.latest.map(|l| l.to_rfc3339()),
        projects: result.meta.projects.clone(),
        models: result.meta.models.clone(),
        sessions: result.meta.sessions.clone(),
        generated_at: "2026-03-25T00:00:00Z".to_string(),
        pricing_date: "2026-03-25".to_string(),
    };
    let dedup = &result.dedup;

    let json_str = format_json(&grouped.data, &grouped.totals, &meta, dedup);

    // Parse it back
    let parsed: serde_json::Value =
        serde_json::from_str(&json_str).expect("JSON output should parse");

    // Assert top-level structure
    assert!(parsed.get("meta").is_some(), "should have meta");
    assert!(parsed.get("data").is_some(), "should have data");
    assert!(parsed.get("totals").is_some(), "should have totals");
    assert!(parsed.get("dedup").is_some(), "should have dedup");

    // meta has expected fields
    let meta_val = parsed.get("meta").unwrap();
    assert!(meta_val.get("dimensions").is_some());
    assert!(meta_val.get("tz").is_some());
    assert!(meta_val.get("order").is_some());
    assert!(meta_val.get("earliest").is_some());
    assert!(meta_val.get("latest").is_some());
    assert!(meta_val.get("projects").is_some());
    assert!(meta_val.get("models").is_some());
    assert!(meta_val.get("sessions").is_some());

    // data is an array
    assert!(parsed["data"].is_array());

    // totals has cost fields
    let totals_val = parsed.get("totals").unwrap();
    assert!(totals_val.get("inputTokens").is_some());
    assert!(totals_val.get("outputTokens").is_some());
    assert!(totals_val.get("totalCost").is_some());

    // dedup has before/after
    let dedup_val = parsed.get("dedup").unwrap();
    assert!(dedup_val.get("before").is_some());
    assert!(dedup_val.get("after").is_some());
}

// ---------------------------------------------------------------------------
// 9. Cost aggregation
// ---------------------------------------------------------------------------

#[test]
fn test_cost_aggregation() {
    let records = vec![
        mock_rec(
            "claude-opus-4-6",
            1000,
            500,
            100,
            200,
            "2026-03-23T10:00:00Z",
            "req_1",
            "msg_1",
        ),
        mock_rec(
            "claude-3-5-haiku-20241022",
            2000,
            800,
            300,
            400,
            "2026-03-23T11:00:00Z",
            "req_2",
            "msg_2",
        ),
    ];
    let dir = make_fixture(&records);
    let opts = default_load_opts(dir.path());
    let result = load_records(&opts);
    let pricing = load_pricing();
    let priced = calculate_cost(&result.records, Some(&pricing));

    let dims = vec![GroupDimension::Day, GroupDimension::Model];
    let group_opts = default_group_opts();
    let grouped = group_records(&priced, &dims, Some(&group_opts));

    assert_eq!(grouped.data.len(), 1, "same day => 1 group");
    let day_group = &grouped.data[0];
    let children = day_group.children.as_ref().unwrap();

    // Parent totalCost must equal sum of children totalCost
    let children_total_cost: f64 = children.iter().map(|c| c.total_cost).sum();
    assert!(
        approx_eq(day_group.total_cost, children_total_cost),
        "parent totalCost ({}) != sum of children totalCost ({})",
        day_group.total_cost,
        children_total_cost,
    );

    // Also check input/output cost sums
    let children_input_cost: f64 = children.iter().map(|c| c.input_cost).sum();
    let children_output_cost: f64 = children.iter().map(|c| c.output_cost).sum();
    assert!(approx_eq(day_group.input_cost, children_input_cost));
    assert!(approx_eq(day_group.output_cost, children_output_cost));

    // Grand totals
    let priced_total_cost: f64 = priced.iter().map(|p| p.total_cost).sum();
    assert!(approx_eq(grouped.totals.total_cost, priced_total_cost));
}

// ---------------------------------------------------------------------------
// 10. loadRecords meta
// ---------------------------------------------------------------------------

#[test]
fn test_load_records_meta() {
    let records = vec![
        mock_rec(
            "claude-opus-4-6",
            100,
            50,
            0,
            0,
            "2026-03-22T08:00:00Z",
            "req_1",
            "msg_1",
        ),
        mock_rec(
            "claude-3-5-haiku-20241022",
            200,
            80,
            0,
            0,
            "2026-03-24T16:00:00Z",
            "req_2",
            "msg_2",
        ),
    ];
    let dir = make_fixture(&records);
    let opts = default_load_opts(dir.path());
    let result = load_records(&opts);

    // earliest / latest
    let earliest = result.meta.earliest.unwrap();
    let latest = result.meta.latest.unwrap();
    assert_eq!(
        earliest.to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
        "2026-03-22T08:00:00Z"
    );
    assert_eq!(
        latest.to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
        "2026-03-24T16:00:00Z"
    );

    // models: sorted unique
    assert_eq!(result.meta.models.len(), 2);
    // BTreeSet produces sorted order
    assert!(result.meta.models.contains(&"claude-opus-4-6".to_string()));
    assert!(result
        .meta
        .models
        .contains(&"claude-3-5-haiku-20241022".to_string()));

    // projects
    assert_eq!(result.meta.projects.len(), 1);
    assert_eq!(result.meta.projects[0], "test-project");

    // sessions
    assert_eq!(result.meta.sessions.len(), 1);
    assert_eq!(result.meta.sessions[0], "session-abc");
}

#[test]
fn test_load_records_meta_empty() {
    // Empty fixture: directory exists but no JSONL files with valid records
    let dir = TempDir::new().unwrap();
    let proj_dir = dir.path().join("projects").join("empty-project");
    fs::create_dir_all(&proj_dir).unwrap();
    // Write an empty file
    fs::write(proj_dir.join("session.jsonl"), "").unwrap();

    let opts = default_load_opts(dir.path());
    let result = load_records(&opts);

    assert!(result.records.is_empty());
    assert!(
        result.meta.earliest.is_none(),
        "empty fixture => None earliest"
    );
    assert!(result.meta.latest.is_none(), "empty fixture => None latest");
    assert!(result.meta.models.is_empty());
    assert!(result.meta.projects.is_empty());
    assert!(result.meta.sessions.is_empty());
}

// ---------------------------------------------------------------------------
// 11. Invalid IANA timezone fallback
// ---------------------------------------------------------------------------

#[test]
fn test_invalid_iana_timezone_no_panic() {
    let records = vec![mock_rec(
        "claude-opus-4-6",
        100,
        50,
        0,
        0,
        "2026-03-23T10:00:00Z",
        "req_1",
        "msg_1",
    )];
    let dir = make_fixture(&records);

    // Use an invalid timezone string
    let opts = LoadOptions {
        claude_dir: Some(dir.path().to_string_lossy().to_string()),
        tz: Some("Not/A_Real_TZ".to_string()),
        ..Default::default()
    };

    // Should not panic
    let result = load_records(&opts);
    assert_eq!(
        result.records.len(),
        1,
        "data should still be produced with invalid tz"
    );

    // Also test grouping with invalid tz
    let pricing = load_pricing();
    let priced = calculate_cost(&result.records, Some(&pricing));
    let dims = vec![GroupDimension::Day];
    let group_opts = GroupOptions {
        order: SortOrder::Asc,
        tz: Some("Not/A_Real_TZ".to_string()),
    };
    let grouped = group_records(&priced, &dims, Some(&group_opts));

    // Should still produce data without panicking
    assert!(!grouped.data.is_empty(), "grouped data should not be empty");
}

// ---------------------------------------------------------------------------
// 12. Project path tests
// ---------------------------------------------------------------------------

#[test]
fn test_hyphenated_project_dir_decoding() {
    // A directory named `-home-username-workspace-test` should decode to
    // `/home/username/workspace/test`
    let records = vec![mock_rec(
        "claude-opus-4-6",
        100,
        50,
        0,
        0,
        "2026-03-23T10:00:00Z",
        "req_1",
        "msg_1",
    )];
    let dir = make_fixture_in_project(
        &records,
        "-home-username-workspace-test",
        "session-abc.jsonl",
    );
    let opts = default_load_opts(dir.path());
    let result = load_records(&opts);

    assert_eq!(result.records.len(), 1);
    assert_eq!(
        result.records[0].project, "/home/username/workspace/test",
        "hyphenated project dir should decode to full path"
    );
}

#[test]
fn test_non_hyphenated_project_name_unchanged() {
    let records = vec![mock_rec(
        "claude-opus-4-6",
        100,
        50,
        0,
        0,
        "2026-03-23T10:00:00Z",
        "req_1",
        "msg_1",
    )];
    let dir = make_fixture_in_project(&records, "my-cool-project", "session-abc.jsonl");
    let opts = default_load_opts(dir.path());
    let result = load_records(&opts);

    assert_eq!(result.records.len(), 1);
    assert_eq!(
        result.records[0].project, "my-cool-project",
        "non-hyphen-prefixed project name should stay unchanged"
    );
}

#[test]
fn test_project_filter_with_decoded_path() {
    let records = vec![mock_rec(
        "claude-opus-4-6",
        100,
        50,
        0,
        0,
        "2026-03-23T10:00:00Z",
        "req_1",
        "msg_1",
    )];
    let dir = make_fixture_in_project(
        &records,
        "-home-username-workspace-test",
        "session-abc.jsonl",
    );

    // Filter by a substring of the decoded path
    let opts = LoadOptions {
        claude_dir: Some(dir.path().to_string_lossy().to_string()),
        tz: Some("UTC".to_string()),
        project: Some("workspace/test".to_string()),
        ..Default::default()
    };

    let result = load_records(&opts);
    assert_eq!(
        result.records.len(),
        1,
        "--project filter should match decoded path"
    );
}

#[test]
fn test_project_filter_excludes_non_matching() {
    let records = vec![mock_rec(
        "claude-opus-4-6",
        100,
        50,
        0,
        0,
        "2026-03-23T10:00:00Z",
        "req_1",
        "msg_1",
    )];
    let dir = make_fixture_in_project(
        &records,
        "-home-username-workspace-test",
        "session-abc.jsonl",
    );

    let opts = LoadOptions {
        claude_dir: Some(dir.path().to_string_lossy().to_string()),
        tz: Some("UTC".to_string()),
        project: Some("nonexistent-project".to_string()),
        ..Default::default()
    };

    let result = load_records(&opts);
    assert_eq!(
        result.records.len(),
        0,
        "--project filter should exclude records from non-matching project"
    );
}

// ---------------------------------------------------------------------------
// Additional edge-case tests
// ---------------------------------------------------------------------------

#[test]
fn test_dedup_via_raw_api() {
    // Test deduplicate_streaming directly with varying output_tokens
    let raw_records: Vec<serde_json::Value> = vec![
        mock_rec(
            "claude-opus-4-6",
            100,
            10,
            0,
            0,
            "2026-03-23T10:00:00Z",
            "req_x",
            "msg_x",
        ),
        mock_rec(
            "claude-opus-4-6",
            100,
            30,
            0,
            0,
            "2026-03-23T10:00:00Z",
            "req_x",
            "msg_x",
        ),
        mock_rec(
            "claude-opus-4-6",
            100,
            50,
            0,
            0,
            "2026-03-23T10:00:00Z",
            "req_x",
            "msg_x",
        ),
    ];

    let (deduped, stats) = deduplicate_streaming(&raw_records);
    assert_eq!(stats.before, 3);
    assert_eq!(stats.after, 1);
    assert_eq!(deduped.len(), 1);

    // The winner should have output_tokens = 50
    let output_tokens = deduped[0]
        .get("message")
        .and_then(|m| m.get("usage"))
        .and_then(|u| u.get("output_tokens"))
        .and_then(|v| v.as_u64())
        .unwrap();
    assert_eq!(output_tokens, 50);
}

#[test]
fn test_dedup_different_keys_not_deduped() {
    // Records with different message IDs should not be deduplicated
    let raw_records: Vec<serde_json::Value> = vec![
        mock_rec(
            "claude-opus-4-6",
            100,
            10,
            0,
            0,
            "2026-03-23T10:00:00Z",
            "req_a",
            "msg_a",
        ),
        mock_rec(
            "claude-opus-4-6",
            100,
            10,
            0,
            0,
            "2026-03-23T10:00:00Z",
            "req_b",
            "msg_b",
        ),
    ];

    let (deduped, stats) = deduplicate_streaming(&raw_records);
    assert_eq!(stats.before, 2);
    assert_eq!(stats.after, 2);
    assert_eq!(deduped.len(), 2);
}

#[test]
fn test_group_records_empty() {
    let priced: Vec<PricedTokenRecord> = vec![];
    let dims = vec![GroupDimension::Day];
    let group_opts = default_group_opts();
    let grouped = group_records(&priced, &dims, Some(&group_opts));

    assert!(grouped.data.is_empty());
    assert_eq!(grouped.totals.input_tokens, 0);
    assert_eq!(grouped.totals.output_tokens, 0);
    assert!(approx_eq(grouped.totals.total_cost, 0.0));
}

#[test]
fn test_shorten_model_name() {
    assert_eq!(shorten_model_name("claude-opus-4-6-20250618"), "opus-4-6");
    assert_eq!(shorten_model_name("claude-3-5-haiku-20241022"), "3-5-haiku");
    assert_eq!(
        shorten_model_name("claude-3-5-sonnet-20241022"),
        "3-5-sonnet"
    );
    assert_eq!(shorten_model_name("some-other-model"), "some-other-model");
}

#[test]
fn test_table_compact_mode() {
    let records = vec![mock_rec(
        "claude-opus-4-6",
        1000,
        500,
        100,
        200,
        "2026-03-23T10:00:00Z",
        "req_1",
        "msg_1",
    )];
    let dir = make_fixture(&records);
    let opts = default_load_opts(dir.path());
    let result = load_records(&opts);
    let pricing = load_pricing();
    let priced = calculate_cost(&result.records, Some(&pricing));
    let dims = vec![GroupDimension::Day];
    let group_opts = default_group_opts();
    let grouped = group_records(&priced, &dims, Some(&group_opts));

    let table_opts = TableOptions {
        dimension_label: "Date".to_string(),
        price_mode: PriceMode::Off,
        compact: true,
        color: Some(false),
    };
    let output = format_table(&grouped.data, &grouped.totals, &table_opts);

    // Compact mode should have "In Total", "Out", "Total" headers but NOT "Cache Cr" / "Cache Rd"
    assert!(output.contains("In Total"));
    assert!(output.contains("Out"));
    assert!(output.contains("Total"));
    assert!(
        !output.contains("Cache Cr"),
        "compact mode should not show Cache Cr"
    );
    assert!(
        !output.contains("Cache Rd"),
        "compact mode should not show Cache Rd"
    );
}

#[test]
fn test_session_filter() {
    // Verify that the session filter works. The session ID is derived from the JSONL filename.
    let records = vec![mock_rec(
        "claude-opus-4-6",
        100,
        50,
        0,
        0,
        "2026-03-23T10:00:00Z",
        "req_1",
        "msg_1",
    )];

    // Create fixture with a specific session filename
    let dir = make_fixture_in_project(&records, "test-project", "my-session-123.jsonl");

    // Filter by substring of the session filename (without extension)
    let opts = LoadOptions {
        claude_dir: Some(dir.path().to_string_lossy().to_string()),
        tz: Some("UTC".to_string()),
        session: Some("my-session".to_string()),
        ..Default::default()
    };
    let result = load_records(&opts);
    assert_eq!(result.records.len(), 1);

    // Non-matching session filter
    let opts_no_match = LoadOptions {
        claude_dir: Some(dir.path().to_string_lossy().to_string()),
        tz: Some("UTC".to_string()),
        session: Some("nonexistent-session".to_string()),
        ..Default::default()
    };
    let result_no_match = load_records(&opts_no_match);
    assert_eq!(result_no_match.records.len(), 0);
}

#[test]
fn test_multiple_sessions_in_meta() {
    // Two different session files in same project
    let dir = TempDir::new().unwrap();
    let proj_dir = dir.path().join("projects").join("test-project");
    fs::create_dir_all(&proj_dir).unwrap();

    let rec1 = mock_rec(
        "claude-opus-4-6",
        100,
        50,
        0,
        0,
        "2026-03-23T10:00:00Z",
        "req_1",
        "msg_1",
    );
    let rec2 = mock_rec(
        "claude-opus-4-6",
        200,
        80,
        0,
        0,
        "2026-03-23T11:00:00Z",
        "req_2",
        "msg_2",
    );

    fs::write(
        proj_dir.join("session-aaa.jsonl"),
        serde_json::to_string(&rec1).unwrap(),
    )
    .unwrap();
    fs::write(
        proj_dir.join("session-bbb.jsonl"),
        serde_json::to_string(&rec2).unwrap(),
    )
    .unwrap();

    let opts = default_load_opts(dir.path());
    let result = load_records(&opts);

    assert_eq!(result.records.len(), 2);
    assert_eq!(result.meta.sessions.len(), 2);
    assert!(result.meta.sessions.contains(&"session-aaa".to_string()));
    assert!(result.meta.sessions.contains(&"session-bbb".to_string()));
}

// ---------------------------------------------------------------------------
// Cross-file dedup (same record in parent + subagents/)
// ---------------------------------------------------------------------------

#[test]
fn test_cross_file_dedup() {
    let dir = TempDir::new().unwrap();
    let proj_dir = dir.path().join("projects").join("test-project");
    let subagent_dir = proj_dir.join("subagents");
    fs::create_dir_all(&subagent_dir).unwrap();

    // Same record appears in both parent session and subagent file
    let rec = mock_rec(
        "claude-opus-4-6",
        100,
        50,
        0,
        0,
        "2026-03-23T10:00:00Z",
        "req_dup",
        "msg_dup",
    );

    fs::write(
        proj_dir.join("session-parent.jsonl"),
        serde_json::to_string(&rec).unwrap(),
    )
    .unwrap();
    fs::write(
        subagent_dir.join("session-sub.jsonl"),
        serde_json::to_string(&rec).unwrap(),
    )
    .unwrap();

    let opts = default_load_opts(dir.path());
    let result = load_records(&opts);

    // Should dedup: 2 raw records → 1 after dedup
    assert_eq!(result.dedup.before, 2);
    assert_eq!(result.dedup.after, 1);
    assert_eq!(result.records.len(), 1);
}

// ---------------------------------------------------------------------------
// Timestamp fallback to file mtime
// ---------------------------------------------------------------------------

#[test]
fn test_timestamp_fallback_to_mtime() {
    let dir = TempDir::new().unwrap();
    let proj_dir = dir.path().join("projects").join("test-project");
    fs::create_dir_all(&proj_dir).unwrap();

    // Record without any timestamp fields
    let rec = serde_json::json!({
        "type": "assistant",
        "requestId": "req_1",
        "message": {
            "id": "msg_1",
            "role": "assistant",
            "model": "claude-opus-4-6",
            "usage": {
                "input_tokens": 100,
                "output_tokens": 50,
                "cache_creation_input_tokens": 0,
                "cache_read_input_tokens": 0,
            }
        }
    });

    fs::write(
        proj_dir.join("session.jsonl"),
        serde_json::to_string(&rec).unwrap(),
    )
    .unwrap();

    let opts = default_load_opts(dir.path());
    let result = load_records(&opts);

    // Record should still be loaded with file mtime as timestamp
    assert_eq!(result.records.len(), 1);
    assert!(
        result.meta.earliest.is_some(),
        "should have a timestamp from mtime"
    );
}

// ---------------------------------------------------------------------------
// <synthetic> model filtering
// ---------------------------------------------------------------------------

#[test]
fn test_synthetic_model_filtered() {
    let records = vec![
        serde_json::json!({
            "type": "assistant",
            "timestamp": "2026-03-23T10:00:00Z",
            "requestId": "req_1",
            "message": {
                "id": "msg_1",
                "model": "<synthetic>",
                "usage": {
                    "input_tokens": 100,
                    "output_tokens": 50,
                    "cache_creation_input_tokens": 0,
                    "cache_read_input_tokens": 0,
                }
            }
        }),
        mock_rec(
            "claude-opus-4-6",
            200,
            80,
            0,
            0,
            "2026-03-23T11:00:00Z",
            "req_2",
            "msg_2",
        ),
    ];
    let dir = make_fixture(&records);
    let opts = default_load_opts(dir.path());
    let result = load_records(&opts);

    // Only the non-synthetic record should pass
    assert_eq!(result.records.len(), 1);
    assert_eq!(result.records[0].model, "claude-opus-4-6");
}

// ---------------------------------------------------------------------------
// Records without type field (should pass through)
// ---------------------------------------------------------------------------

#[test]
fn test_no_type_field_passes() {
    let rec = serde_json::json!({
        "timestamp": "2026-03-23T10:00:00Z",
        "requestId": "req_1",
        "message": {
            "id": "msg_1",
            "model": "claude-opus-4-6",
            "usage": {
                "input_tokens": 100,
                "output_tokens": 50,
                "cache_creation_input_tokens": 0,
                "cache_read_input_tokens": 0,
            }
        }
    });
    let dir = make_fixture(&[rec]);
    let opts = default_load_opts(dir.path());
    let result = load_records(&opts);

    assert_eq!(
        result.records.len(),
        1,
        "record without type field should pass"
    );
}

// ---------------------------------------------------------------------------
// Records with invalid type should be filtered
// ---------------------------------------------------------------------------

#[test]
fn test_invalid_type_filtered() {
    let rec = serde_json::json!({
        "type": "system",
        "timestamp": "2026-03-23T10:00:00Z",
        "requestId": "req_1",
        "message": {
            "id": "msg_1",
            "model": "claude-opus-4-6",
            "usage": {
                "input_tokens": 100,
                "output_tokens": 50,
                "cache_creation_input_tokens": 0,
                "cache_read_input_tokens": 0,
            }
        }
    });
    let dir = make_fixture(&[rec]);
    let opts = default_load_opts(dir.path());
    let result = load_records(&opts);

    assert_eq!(
        result.records.len(),
        0,
        "type=system should be filtered out"
    );
}

// ---------------------------------------------------------------------------
// Records missing usage fields should be filtered
// ---------------------------------------------------------------------------

#[test]
fn test_missing_usage_filtered() {
    let rec = serde_json::json!({
        "type": "assistant",
        "timestamp": "2026-03-23T10:00:00Z",
        "requestId": "req_1",
        "message": {
            "id": "msg_1",
            "model": "claude-opus-4-6",
        }
    });
    let dir = make_fixture(&[rec]);
    let opts = default_load_opts(dir.path());
    let result = load_records(&opts);

    assert_eq!(
        result.records.len(),
        0,
        "records without usage should be filtered"
    );
}

// ---------------------------------------------------------------------------
// Records missing model should be filtered
// ---------------------------------------------------------------------------

#[test]
fn test_missing_model_filtered() {
    let rec = serde_json::json!({
        "type": "assistant",
        "timestamp": "2026-03-23T10:00:00Z",
        "requestId": "req_1",
        "message": {
            "id": "msg_1",
            "usage": {
                "input_tokens": 100,
                "output_tokens": 50,
                "cache_creation_input_tokens": 0,
                "cache_read_input_tokens": 0,
            }
        }
    });
    let dir = make_fixture(&[rec]);
    let opts = default_load_opts(dir.path());
    let result = load_records(&opts);

    assert_eq!(
        result.records.len(),
        0,
        "records without model should be filtered"
    );
}

// ---------------------------------------------------------------------------
// Numeric timestamp in full pipeline
// ---------------------------------------------------------------------------

#[test]
fn test_numeric_timestamp_in_pipeline() {
    // 1711180800000 ms = 2024-03-23T08:00:00Z
    let rec = serde_json::json!({
        "type": "assistant",
        "timestamp": 1711180800000u64,
        "requestId": "req_1",
        "message": {
            "id": "msg_1",
            "model": "claude-opus-4-6",
            "usage": {
                "input_tokens": 100,
                "output_tokens": 50,
                "cache_creation_input_tokens": 0,
                "cache_read_input_tokens": 0,
            }
        }
    });
    let dir = make_fixture(&[rec]);
    let opts = LoadOptions {
        claude_dir: Some(dir.path().to_string_lossy().to_string()),
        tz: Some("UTC".to_string()),
        ..Default::default()
    };
    let result = load_records(&opts);

    assert_eq!(result.records.len(), 1);
    assert_eq!(
        result
            .meta
            .earliest
            .unwrap()
            .to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
        "2024-03-23T08:00:00Z"
    );
}

// ---------------------------------------------------------------------------
// Numeric timestamp in seconds (not millis)
// ---------------------------------------------------------------------------

#[test]
fn test_numeric_timestamp_seconds_in_pipeline() {
    // 1711180800 seconds = 2024-03-23T08:00:00Z
    let rec = serde_json::json!({
        "type": "assistant",
        "timestamp": 1711180800u64,
        "requestId": "req_1",
        "message": {
            "id": "msg_1",
            "model": "claude-opus-4-6",
            "usage": {
                "input_tokens": 100,
                "output_tokens": 50,
                "cache_creation_input_tokens": 0,
                "cache_read_input_tokens": 0,
            }
        }
    });
    let dir = make_fixture(&[rec]);
    let opts = LoadOptions {
        claude_dir: Some(dir.path().to_string_lossy().to_string()),
        tz: Some("UTC".to_string()),
        ..Default::default()
    };
    let result = load_records(&opts);

    assert_eq!(result.records.len(), 1);
    assert_eq!(
        result
            .meta
            .earliest
            .unwrap()
            .to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
        "2024-03-23T08:00:00Z"
    );
}

// ---------------------------------------------------------------------------
// Formatter: hierarchy rendering (parent→child)
// ---------------------------------------------------------------------------

fn make_hierarchy_data() -> (Vec<GroupedData>, GroupedData) {
    let child1 = GroupedData {
        label: "opus-4-6".to_string(),
        input_tokens: 100,
        output_tokens: 50,
        cache_creation_tokens: 10,
        cache_read_tokens: 20,
        input_cost: 0.01,
        cache_creation_cost: 0.002,
        cache_read_cost: 0.003,
        output_cost: 0.005,
        total_cost: 0.02,
        children: None,
    };
    let child2 = GroupedData {
        label: "3-5-haiku".to_string(),
        input_tokens: 200,
        output_tokens: 80,
        cache_creation_tokens: 30,
        cache_read_tokens: 40,
        input_cost: 0.001,
        cache_creation_cost: 0.0005,
        cache_read_cost: 0.0003,
        output_cost: 0.002,
        total_cost: 0.004,
        children: None,
    };
    let parent = GroupedData {
        label: "2026-03-23".to_string(),
        input_tokens: 300,
        output_tokens: 130,
        cache_creation_tokens: 40,
        cache_read_tokens: 60,
        input_cost: 0.011,
        cache_creation_cost: 0.0025,
        cache_read_cost: 0.0033,
        output_cost: 0.007,
        total_cost: 0.024,
        children: Some(vec![child1.clone(), child2.clone()]),
    };
    let totals = GroupedData {
        label: "TOTAL".to_string(),
        input_tokens: 300,
        output_tokens: 130,
        cache_creation_tokens: 40,
        cache_read_tokens: 60,
        input_cost: 0.011,
        cache_creation_cost: 0.0025,
        cache_read_cost: 0.0033,
        output_cost: 0.007,
        total_cost: 0.024,
        children: Some(vec![child1, child2]),
    };
    (vec![parent], totals)
}

#[test]
fn test_table_hierarchy_rendering() {
    use ccost::formatters::table::TableOptions;

    let (data, totals) = make_hierarchy_data();
    let opts = TableOptions {
        dimension_label: "Date/Model".to_string(),
        price_mode: PriceMode::Decimal,
        compact: false,
        color: Some(false),
    };
    let output = format_table(&data, &totals, &opts);

    assert!(output.contains("2026-03-23"), "should contain parent label");
    assert!(
        output.contains("└─ opus-4-6"),
        "should contain child with prefix"
    );
    assert!(
        output.contains("└─ 3-5-haiku"),
        "should contain second child"
    );
    assert!(output.contains("TOTAL"), "should contain totals");
}

#[test]
fn test_markdown_hierarchy_rendering() {
    use ccost::formatters::markdown::MarkdownOptions;

    let (data, totals) = make_hierarchy_data();
    let opts = MarkdownOptions {
        dimension_label: "Date/Model".to_string(),
        price_mode: PriceMode::Integer,
        compact: false,
    };
    let output = format_markdown(&data, &totals, &opts);

    assert!(output.contains("2026-03-23"), "should contain parent label");
    assert!(output.contains("└─ opus-4-6"), "should contain child");
    assert!(
        output.contains("└─ 3-5-haiku"),
        "should contain second child"
    );
    assert!(output.contains("TOTAL"), "should contain totals");
    assert!(output.contains("|"), "should be markdown table");
}

#[test]
fn test_html_hierarchy_rendering() {
    use ccost::formatters::html::HtmlOptions;

    let (data, totals) = make_hierarchy_data();
    let opts = HtmlOptions {
        dimension_label: "Date/Model".to_string(),
        price_mode: PriceMode::Integer,
        compact: false,
        title: None,
    };
    let output = format_html(&data, &totals, &opts);

    assert!(output.contains("2026-03-23"), "should contain parent label");
    assert!(output.contains("opus-4-6"), "should contain child model");
    assert!(output.contains("3-5-haiku"), "should contain second child");
    assert!(output.contains("TOTAL"), "should contain totals");
    assert!(output.contains("<!DOCTYPE html>"), "should be valid HTML");
}

#[test]
fn test_csv_hierarchy_rendering() {
    use ccost::formatters::csv::DsvOptions;

    let (data, totals) = make_hierarchy_data();
    let opts = DsvOptions {
        dimension_label: "Date/Model".to_string(),
        price_mode: PriceMode::Off,
        compact: false,
    };
    let output = format_csv(&data, &totals, &opts);

    assert!(output.contains("2026-03-23"), "should contain parent label");
    assert!(
        output.contains("└─ opus-4-6"),
        "should contain child with prefix"
    );
    assert!(output.contains("TOTAL"), "should contain totals");
    // Count data rows (header + parent + 2 children + totals + 2 totals children = 7 lines)
    assert_eq!(output.lines().count(), 7);
}

#[test]
fn test_tsv_hierarchy_rendering() {
    use ccost::formatters::csv::DsvOptions;

    let (data, totals) = make_hierarchy_data();
    let opts = DsvOptions {
        dimension_label: "Date/Model".to_string(),
        price_mode: PriceMode::Off,
        compact: false,
    };
    let output = format_tsv(&data, &totals, &opts);

    assert!(output.contains("2026-03-23"));
    assert!(output.contains("└─ opus-4-6"));
    assert!(output.contains("\t"), "should be tab-separated");
}

// ---------------------------------------------------------------------------
// Formatter: compact mode
// ---------------------------------------------------------------------------

#[test]
fn test_markdown_compact() {
    use ccost::formatters::markdown::MarkdownOptions;

    let (data, totals) = make_hierarchy_data();
    let opts = MarkdownOptions {
        dimension_label: "Date".to_string(),
        price_mode: PriceMode::Off,
        compact: true,
    };
    let output = format_markdown(&data, &totals, &opts);

    assert!(!output.contains("Cache Cr"), "compact should hide Cache Cr");
    assert!(!output.contains("Cache Rd"), "compact should hide Cache Rd");
    assert!(output.contains("In Total"));
    assert!(output.contains("Out"));
    assert!(output.contains("Total"));
}

#[test]
fn test_html_compact() {
    use ccost::formatters::html::HtmlOptions;

    let (data, totals) = make_hierarchy_data();
    let opts = HtmlOptions {
        dimension_label: "Date".to_string(),
        price_mode: PriceMode::Off,
        compact: true,
        title: None,
    };
    let output = format_html(&data, &totals, &opts);

    assert!(
        !output.contains("Cache Creation"),
        "compact HTML should hide Cache Creation"
    );
    assert!(output.contains("Input Total"));
}

#[test]
fn test_csv_compact() {
    use ccost::formatters::csv::DsvOptions;

    let (data, totals) = make_hierarchy_data();
    let opts = DsvOptions {
        dimension_label: "Date".to_string(),
        price_mode: PriceMode::Off,
        compact: true,
    };
    let output = format_csv(&data, &totals, &opts);
    let header = output.lines().next().unwrap();

    assert!(
        !header.contains("Cache"),
        "compact CSV should not have Cache columns"
    );
    assert_eq!(header.split(',').count(), 4, "compact: label + 3 columns");
}

#[test]
fn test_csv_non_compact() {
    use ccost::formatters::csv::DsvOptions;

    let (data, totals) = make_hierarchy_data();
    let opts = DsvOptions {
        dimension_label: "Date".to_string(),
        price_mode: PriceMode::Off,
        compact: false,
    };
    let output = format_csv(&data, &totals, &opts);
    let header = output.lines().next().unwrap();

    assert!(
        header.contains("Cache Cr"),
        "non-compact CSV should have Cache Cr"
    );
    assert_eq!(
        header.split(',').count(),
        7,
        "non-compact: label + 6 columns"
    );
}

// ---------------------------------------------------------------------------
// Formatter: price modes
// ---------------------------------------------------------------------------

#[test]
fn test_markdown_price_decimal() {
    use ccost::formatters::markdown::MarkdownOptions;

    let (data, totals) = make_hierarchy_data();
    let opts = MarkdownOptions {
        dimension_label: "Date".to_string(),
        price_mode: PriceMode::Decimal,
        compact: false,
    };
    let output = format_markdown(&data, &totals, &opts);

    assert!(output.contains("$"), "decimal mode should contain '$'");
    assert!(output.contains("."), "decimal mode should contain '.'");
}

#[test]
fn test_markdown_price_off() {
    use ccost::formatters::markdown::MarkdownOptions;

    let (data, totals) = make_hierarchy_data();
    let opts = MarkdownOptions {
        dimension_label: "Date".to_string(),
        price_mode: PriceMode::Off,
        compact: false,
    };
    let output = format_markdown(&data, &totals, &opts);

    assert!(!output.contains("$"), "off mode should not contain '$'");
}

#[test]
fn test_csv_price_integer() {
    use ccost::formatters::csv::DsvOptions;

    let (data, totals) = make_hierarchy_data();
    let opts = DsvOptions {
        dimension_label: "Date".to_string(),
        price_mode: PriceMode::Integer,
        compact: false,
    };
    let output = format_csv(&data, &totals, &opts);

    assert!(output.contains("$"), "integer mode should contain '$'");
}

#[test]
fn test_html_price_decimal() {
    use ccost::formatters::html::HtmlOptions;

    let (data, totals) = make_hierarchy_data();
    let opts = HtmlOptions {
        dimension_label: "Date".to_string(),
        price_mode: PriceMode::Decimal,
        compact: false,
        title: None,
    };
    let output = format_html(&data, &totals, &opts);

    assert!(output.contains("$"), "decimal HTML should contain '$'");
}

// ---------------------------------------------------------------------------
// Formatter: empty data
// ---------------------------------------------------------------------------

#[test]
fn test_table_empty_data() {
    use ccost::formatters::table::TableOptions;

    let totals = GroupedData {
        label: "TOTAL".to_string(),
        input_tokens: 0,
        output_tokens: 0,
        cache_creation_tokens: 0,
        cache_read_tokens: 0,
        input_cost: 0.0,
        cache_creation_cost: 0.0,
        cache_read_cost: 0.0,
        output_cost: 0.0,
        total_cost: 0.0,
        children: None,
    };
    let opts = TableOptions {
        dimension_label: "Date".to_string(),
        price_mode: PriceMode::Off,
        compact: false,
        color: Some(false),
    };
    let output = format_table(&[], &totals, &opts);
    assert!(
        output.contains("TOTAL"),
        "empty data should still have totals"
    );
}

#[test]
fn test_markdown_empty_data() {
    use ccost::formatters::markdown::MarkdownOptions;

    let totals = GroupedData {
        label: "TOTAL".to_string(),
        input_tokens: 0,
        output_tokens: 0,
        cache_creation_tokens: 0,
        cache_read_tokens: 0,
        input_cost: 0.0,
        cache_creation_cost: 0.0,
        cache_read_cost: 0.0,
        output_cost: 0.0,
        total_cost: 0.0,
        children: None,
    };
    let opts = MarkdownOptions {
        dimension_label: "Date".to_string(),
        price_mode: PriceMode::Off,
        compact: false,
    };
    let output = format_markdown(&[], &totals, &opts);
    assert!(output.contains("TOTAL"));
}

#[test]
fn test_csv_empty_data() {
    use ccost::formatters::csv::DsvOptions;

    let totals = GroupedData {
        label: "TOTAL".to_string(),
        input_tokens: 0,
        output_tokens: 0,
        cache_creation_tokens: 0,
        cache_read_tokens: 0,
        input_cost: 0.0,
        cache_creation_cost: 0.0,
        cache_read_cost: 0.0,
        output_cost: 0.0,
        total_cost: 0.0,
        children: None,
    };
    let opts = DsvOptions {
        dimension_label: "Date".to_string(),
        price_mode: PriceMode::Off,
        compact: false,
    };
    let output = format_csv(&[], &totals, &opts);
    // Header + TOTAL row = 2 lines
    assert_eq!(output.lines().count(), 2);
    assert!(output.contains("TOTAL"));
}

// ---------------------------------------------------------------------------
// Table: compact mode hides cache columns
// ---------------------------------------------------------------------------

#[test]
fn test_table_compact_hides_cache() {
    use ccost::formatters::table::TableOptions;

    let (data, totals) = make_hierarchy_data();
    let opts = TableOptions {
        dimension_label: "Date".to_string(),
        price_mode: PriceMode::Off,
        compact: true,
        color: Some(false),
    };
    let output = format_table(&data, &totals, &opts);

    assert!(
        !output.contains("Cache Cr"),
        "compact table should hide Cache Cr"
    );
    assert!(
        !output.contains("Cache Rd"),
        "compact table should hide Cache Rd"
    );
    assert!(output.contains("In Total"));
}

// ---------------------------------------------------------------------------
// Table: color mode
// ---------------------------------------------------------------------------

#[test]
fn test_table_with_color() {
    use ccost::formatters::table::TableOptions;

    let (data, totals) = make_hierarchy_data();
    let opts = TableOptions {
        dimension_label: "Date".to_string(),
        price_mode: PriceMode::Integer,
        compact: false,
        color: Some(true),
    };
    let output = format_table(&data, &totals, &opts);

    // ANSI escape codes for green on TOTAL row
    assert!(
        output.contains("\x1b["),
        "color=true should contain ANSI codes"
    );
}

#[test]
fn test_table_without_color() {
    use ccost::formatters::table::TableOptions;

    let (data, totals) = make_hierarchy_data();
    let opts = TableOptions {
        dimension_label: "Date".to_string(),
        price_mode: PriceMode::Integer,
        compact: false,
        color: Some(false),
    };
    let output = format_table(&data, &totals, &opts);

    assert!(
        !output.contains("\x1b["),
        "color=false should not contain ANSI codes"
    );
}

// ---------------------------------------------------------------------------
// JSON formatter: edge cases
// ---------------------------------------------------------------------------

#[test]
fn test_json_with_children() {
    use ccost::formatters::json::JsonMeta;

    let (data, totals) = make_hierarchy_data();
    let meta = JsonMeta {
        dimensions: vec!["day".to_string(), "model".to_string()],
        from: Some("2026-03-01".to_string()),
        to: Some("2026-03-31".to_string()),
        tz: Some("UTC".to_string()),
        project: None,
        model: None,
        session: None,
        order: "asc".to_string(),
        earliest: Some("2026-03-23T10:00:00Z".to_string()),
        latest: Some("2026-03-23T11:00:00Z".to_string()),
        projects: vec!["test-project".to_string()],
        models: vec!["claude-opus-4-6".to_string()],
        sessions: vec!["session-abc".to_string()],
        generated_at: "2026-03-25T00:00:00Z".to_string(),
        pricing_date: "2026-03-25".to_string(),
    };
    let dedup = DedupStats {
        before: 10,
        after: 8,
    };

    let json_str = format_json(&data, &totals, &meta, &dedup);
    let parsed: serde_json::Value = serde_json::from_str(&json_str).unwrap();

    // Data should have children
    let first_data = &parsed["data"][0];
    assert!(
        first_data["children"].is_array(),
        "data entry should have children"
    );
    assert_eq!(first_data["children"].as_array().unwrap().len(), 2);

    // Totals should have children
    assert!(parsed["totals"]["children"].is_array());

    // Meta should have from/to
    assert_eq!(parsed["meta"]["from"].as_str().unwrap(), "2026-03-01");
    assert_eq!(parsed["meta"]["to"].as_str().unwrap(), "2026-03-31");

    // Dedup values
    assert_eq!(parsed["dedup"]["before"].as_u64().unwrap(), 10);
    assert_eq!(parsed["dedup"]["after"].as_u64().unwrap(), 8);
}

// ---------------------------------------------------------------------------
// HTML: custom title
// ---------------------------------------------------------------------------

#[test]
fn test_html_custom_title() {
    use ccost::formatters::html::HtmlOptions;

    let (data, totals) = make_hierarchy_data();
    let opts = HtmlOptions {
        dimension_label: "Date".to_string(),
        price_mode: PriceMode::Off,
        compact: false,
        title: Some("My Custom Report".to_string()),
    };
    let output = format_html(&data, &totals, &opts);

    assert!(
        output.contains("My Custom Report"),
        "should contain custom title"
    );
}

// ---------------------------------------------------------------------------
// Grouping: single dimension (project, session, model, hour, month)
// ---------------------------------------------------------------------------

#[test]
fn test_group_by_project() {
    let records = vec![mock_rec(
        "claude-opus-4-6",
        100,
        50,
        0,
        0,
        "2026-03-23T10:00:00Z",
        "req_1",
        "msg_1",
    )];
    let dir = make_fixture(&records);
    let opts = default_load_opts(dir.path());
    let result = load_records(&opts);
    let pricing = load_pricing();
    let priced = calculate_cost(&result.records, Some(&pricing));

    let dims = vec![GroupDimension::Project];
    let group_opts = default_group_opts();
    let grouped = group_records(&priced, &dims, Some(&group_opts));

    assert_eq!(grouped.data.len(), 1);
    assert_eq!(grouped.data[0].label, "test-project");
}

#[test]
fn test_group_by_session() {
    let records = vec![mock_rec(
        "claude-opus-4-6",
        100,
        50,
        0,
        0,
        "2026-03-23T10:00:00Z",
        "req_1",
        "msg_1",
    )];
    let dir = make_fixture(&records);
    let opts = default_load_opts(dir.path());
    let result = load_records(&opts);
    let pricing = load_pricing();
    let priced = calculate_cost(&result.records, Some(&pricing));

    let dims = vec![GroupDimension::Session];
    let group_opts = default_group_opts();
    let grouped = group_records(&priced, &dims, Some(&group_opts));

    assert_eq!(grouped.data.len(), 1);
    assert_eq!(grouped.data[0].label, "session-abc");
}

#[test]
fn test_group_by_model() {
    let records = vec![
        mock_rec(
            "claude-opus-4-6",
            100,
            50,
            0,
            0,
            "2026-03-23T10:00:00Z",
            "req_1",
            "msg_1",
        ),
        mock_rec(
            "claude-3-5-haiku-20241022",
            200,
            80,
            0,
            0,
            "2026-03-23T11:00:00Z",
            "req_2",
            "msg_2",
        ),
    ];
    let dir = make_fixture(&records);
    let opts = default_load_opts(dir.path());
    let result = load_records(&opts);
    let pricing = load_pricing();
    let priced = calculate_cost(&result.records, Some(&pricing));

    let dims = vec![GroupDimension::Model];
    let group_opts = default_group_opts();
    let grouped = group_records(&priced, &dims, Some(&group_opts));

    assert_eq!(grouped.data.len(), 2);
    let labels: Vec<&str> = grouped.data.iter().map(|d| d.label.as_str()).collect();
    assert!(
        labels.contains(&"opus-4-6"),
        "model name should be shortened"
    );
    assert!(
        labels.contains(&"3-5-haiku"),
        "model name should strip date suffix"
    );
}

#[test]
fn test_group_by_hour() {
    let records = vec![
        mock_rec(
            "claude-opus-4-6",
            100,
            50,
            0,
            0,
            "2026-03-23T10:30:00Z",
            "req_1",
            "msg_1",
        ),
        mock_rec(
            "claude-opus-4-6",
            200,
            80,
            0,
            0,
            "2026-03-23T10:45:00Z",
            "req_2",
            "msg_2",
        ),
        mock_rec(
            "claude-opus-4-6",
            300,
            120,
            0,
            0,
            "2026-03-23T11:15:00Z",
            "req_3",
            "msg_3",
        ),
    ];
    let dir = make_fixture(&records);
    let opts = default_load_opts(dir.path());
    let result = load_records(&opts);
    let pricing = load_pricing();
    let priced = calculate_cost(&result.records, Some(&pricing));

    let dims = vec![GroupDimension::Hour];
    let group_opts = default_group_opts();
    let grouped = group_records(&priced, &dims, Some(&group_opts));

    assert_eq!(grouped.data.len(), 2, "should group into 2 hours");
    assert_eq!(grouped.data[0].label, "2026-03-23 10:00");
    assert_eq!(grouped.data[1].label, "2026-03-23 11:00");
    // First hour should have 2 records aggregated
    assert_eq!(grouped.data[0].input_tokens, 300);
}

#[test]
fn test_group_by_month() {
    let records = vec![
        mock_rec(
            "claude-opus-4-6",
            100,
            50,
            0,
            0,
            "2026-03-23T10:00:00Z",
            "req_1",
            "msg_1",
        ),
        mock_rec(
            "claude-opus-4-6",
            200,
            80,
            0,
            0,
            "2026-04-05T10:00:00Z",
            "req_2",
            "msg_2",
        ),
    ];
    let dir = make_fixture(&records);
    let opts = default_load_opts(dir.path());
    let result = load_records(&opts);
    let pricing = load_pricing();
    let priced = calculate_cost(&result.records, Some(&pricing));

    let dims = vec![GroupDimension::Month];
    let group_opts = default_group_opts();
    let grouped = group_records(&priced, &dims, Some(&group_opts));

    assert_eq!(grouped.data.len(), 2);
    assert_eq!(grouped.data[0].label, "2026-03");
    assert_eq!(grouped.data[1].label, "2026-04");
}

// ---------------------------------------------------------------------------
// Subagent dimension: agent_id extraction and grouping
// ---------------------------------------------------------------------------

#[test]
fn test_subagent_agent_id_extracted_new_structure() {
    let dir = TempDir::new().unwrap();
    let proj_dir = dir.path().join("projects").join("test-project");
    let session_uuid = "abc12345-1234-5678-9abc-def012345678";
    let subagent_dir = proj_dir.join(session_uuid).join("subagents");
    fs::create_dir_all(&subagent_dir).unwrap();

    // Main session file
    let main_rec = mock_rec(
        "claude-opus-4-6",
        100,
        50,
        0,
        0,
        "2026-03-23T10:00:00Z",
        "req_main",
        "msg_main",
    );
    fs::write(
        proj_dir.join(format!("{}.jsonl", session_uuid)),
        serde_json::to_string(&main_rec).unwrap(),
    )
    .unwrap();

    // Subagent file
    let sub_rec = mock_rec(
        "claude-opus-4-6",
        200,
        80,
        0,
        0,
        "2026-03-23T10:05:00Z",
        "req_sub",
        "msg_sub",
    );
    fs::write(
        subagent_dir.join("agent-abc123.jsonl"),
        serde_json::to_string(&sub_rec).unwrap(),
    )
    .unwrap();

    let opts = default_load_opts(dir.path());
    let result = load_records(&opts);

    assert_eq!(result.records.len(), 2);

    let main = result.records.iter().find(|r| r.agent_id.is_empty());
    let sub = result.records.iter().find(|r| !r.agent_id.is_empty());
    assert!(main.is_some(), "should have a main session record");
    assert!(sub.is_some(), "should have a subagent record");
    assert_eq!(sub.unwrap().agent_id, "agent-abc123");
    // Both share the same session_id
    assert_eq!(main.unwrap().session_id, session_uuid);
    assert_eq!(sub.unwrap().session_id, session_uuid);
}

#[test]
fn test_group_by_subagent_dimension() {
    let dir = TempDir::new().unwrap();
    let proj_dir = dir.path().join("projects").join("test-project");
    let session_uuid = "abc12345-1234-5678-9abc-def012345678";
    let subagent_dir = proj_dir.join(session_uuid).join("subagents");
    fs::create_dir_all(&subagent_dir).unwrap();

    // Main session file
    let main_rec = mock_rec(
        "claude-opus-4-6",
        100,
        50,
        0,
        0,
        "2026-03-23T10:00:00Z",
        "req_main",
        "msg_main",
    );
    fs::write(
        proj_dir.join(format!("{}.jsonl", session_uuid)),
        serde_json::to_string(&main_rec).unwrap(),
    )
    .unwrap();

    // Two different subagent files
    let sub1 = mock_rec(
        "claude-opus-4-6",
        200,
        80,
        0,
        0,
        "2026-03-23T10:05:00Z",
        "req_sub1",
        "msg_sub1",
    );
    let sub2 = mock_rec(
        "claude-opus-4-6",
        300,
        120,
        0,
        0,
        "2026-03-23T10:10:00Z",
        "req_sub2",
        "msg_sub2",
    );
    fs::write(
        subagent_dir.join("agent-explorer.jsonl"),
        serde_json::to_string(&sub1).unwrap(),
    )
    .unwrap();
    fs::write(
        subagent_dir.join("agent-planner.jsonl"),
        serde_json::to_string(&sub2).unwrap(),
    )
    .unwrap();

    let opts = default_load_opts(dir.path());
    let result = load_records(&opts);
    let pricing = load_pricing();
    let priced = calculate_cost(&result.records, Some(&pricing));

    let dims = vec![GroupDimension::Subagent];
    let group_opts = default_group_opts();
    let grouped = group_records(&priced, &dims, Some(&group_opts));

    // 3 groups: (main), agent-explorer, agent-planner
    assert_eq!(grouped.data.len(), 3);
    let labels: Vec<&str> = grouped.data.iter().map(|d| d.label.as_str()).collect();
    assert!(labels.contains(&"(main)"));
    assert!(labels.contains(&"agent-explorer"));
    assert!(labels.contains(&"agent-planner"));

    let main_group = grouped.data.iter().find(|d| d.label == "(main)").unwrap();
    assert_eq!(main_group.input_tokens, 100);

    let explorer = grouped
        .data
        .iter()
        .find(|d| d.label == "agent-explorer")
        .unwrap();
    assert_eq!(explorer.input_tokens, 200);

    let planner = grouped
        .data
        .iter()
        .find(|d| d.label == "agent-planner")
        .unwrap();
    assert_eq!(planner.input_tokens, 300);
}

#[test]
fn test_two_level_session_subagent_grouping() {
    let dir = TempDir::new().unwrap();
    let proj_dir = dir.path().join("projects").join("test-project");
    let session_uuid = "abc12345-1234-5678-9abc-def012345678";
    let subagent_dir = proj_dir.join(session_uuid).join("subagents");
    fs::create_dir_all(&subagent_dir).unwrap();

    let main_rec = mock_rec(
        "claude-opus-4-6",
        100,
        50,
        0,
        0,
        "2026-03-23T10:00:00Z",
        "req_main",
        "msg_main",
    );
    fs::write(
        proj_dir.join(format!("{}.jsonl", session_uuid)),
        serde_json::to_string(&main_rec).unwrap(),
    )
    .unwrap();

    let sub_rec = mock_rec(
        "claude-opus-4-6",
        200,
        80,
        0,
        0,
        "2026-03-23T10:05:00Z",
        "req_sub",
        "msg_sub",
    );
    fs::write(
        subagent_dir.join("agent-abc.jsonl"),
        serde_json::to_string(&sub_rec).unwrap(),
    )
    .unwrap();

    let opts = default_load_opts(dir.path());
    let result = load_records(&opts);
    let pricing = load_pricing();
    let priced = calculate_cost(&result.records, Some(&pricing));

    // Two-level: session > subagent
    let dims = vec![GroupDimension::Session, GroupDimension::Subagent];
    let group_opts = default_group_opts();
    let grouped = group_records(&priced, &dims, Some(&group_opts));

    // One session group with two children
    assert_eq!(grouped.data.len(), 1);
    assert_eq!(grouped.data[0].label, session_uuid);
    let children = grouped.data[0].children.as_ref().unwrap();
    assert_eq!(children.len(), 2);
    let child_labels: Vec<&str> = children.iter().map(|c| c.label.as_str()).collect();
    assert!(child_labels.contains(&"(main)"));
    assert!(child_labels.contains(&"agent-abc"));
}

// ---------------------------------------------------------------------------
// Per-tool grouping
// ---------------------------------------------------------------------------

/// Build a mock assistant record with tool_use blocks.
fn mock_rec_with_tools(
    model: &str,
    input: u64,
    output: u64,
    ts: &str,
    req_id: &str,
    msg_id: &str,
    tools: &[&str],
) -> serde_json::Value {
    let mut content: Vec<serde_json::Value> = tools
        .iter()
        .enumerate()
        .map(|(i, name)| {
            serde_json::json!({
                "type": "tool_use",
                "id": format!("toolu_{:04}", i),
                "name": name,
                "input": {}
            })
        })
        .collect();
    // Also add a text block to be realistic
    content.push(serde_json::json!({"type": "text", "text": "some response"}));

    serde_json::json!({
        "timestamp": ts,
        "type": "assistant",
        "sessionId": "session-abc",
        "message": {
            "id": msg_id,
            "role": "assistant",
            "model": model,
            "content": content,
            "usage": {
                "input_tokens": input,
                "output_tokens": output,
                "cache_creation_input_tokens": 0,
                "cache_read_input_tokens": 0,
            }
        },
        "requestId": req_id,
    })
}

/// Mock user text message (line boundary, no usage).
fn mock_user_text(ts: &str, text: &str) -> serde_json::Value {
    serde_json::json!({
        "timestamp": ts,
        "type": "user",
        "message": { "role": "user", "content": text }
    })
}

/// Mock user tool_result message (no usage).
fn mock_user_tool_result(ts: &str, tool_use_id: &str) -> serde_json::Value {
    serde_json::json!({
        "timestamp": ts,
        "type": "user",
        "message": {
            "role": "user",
            "content": [{"type": "tool_result", "tool_use_id": tool_use_id, "content": "ok"}]
        }
    })
}

#[test]
fn test_per_tool_grouping() {
    let records = vec![
        // text-only assistant message
        mock_rec(
            "claude-opus-4-6",
            1000,
            500,
            0,
            0,
            "2026-01-15T10:00:00Z",
            "r1",
            "m1",
        ),
        // assistant using Read
        mock_rec_with_tools(
            "claude-opus-4-6",
            2000,
            800,
            "2026-01-15T10:01:00Z",
            "r2",
            "m2",
            &["Read"],
        ),
        // assistant using Edit
        mock_rec_with_tools(
            "claude-opus-4-6",
            1500,
            600,
            "2026-01-15T10:02:00Z",
            "r3",
            "m3",
            &["Edit"],
        ),
        // assistant using Read again
        mock_rec_with_tools(
            "claude-opus-4-6",
            3000,
            1000,
            "2026-01-15T10:03:00Z",
            "r4",
            "m4",
            &["Read"],
        ),
    ];

    let dir = make_fixture(&records);
    let opts = default_load_opts(dir.path());
    let loaded = load_records(&opts);
    assert_eq!(loaded.records.len(), 4);

    let priced = calculate_cost(&loaded.records, None);
    let dims = vec![GroupDimension::Tool];
    let grouped = group_records(&priced, &dims, Some(&default_group_opts()));

    // Should have 3 groups: (text), Edit, Read
    assert_eq!(grouped.data.len(), 3);
    let labels: Vec<&str> = grouped.data.iter().map(|g| g.label.as_str()).collect();
    assert!(labels.contains(&"(text)"));
    assert!(labels.contains(&"Edit"));
    assert!(labels.contains(&"Read"));

    // Read group should have 2 records worth of tokens
    let read_group = grouped.data.iter().find(|g| g.label == "Read").unwrap();
    assert_eq!(read_group.input_tokens, 2000 + 3000);
    assert_eq!(read_group.output_tokens, 800 + 1000);
}

#[test]
fn test_per_tool_multi_tool_message() {
    let records = vec![mock_rec_with_tools(
        "claude-opus-4-6",
        1000,
        500,
        "2026-01-15T10:00:00Z",
        "r1",
        "m1",
        &["Read", "Edit"],
    )];

    let dir = make_fixture(&records);
    let opts = default_load_opts(dir.path());
    let loaded = load_records(&opts);
    assert_eq!(loaded.records.len(), 1);
    // tool_names should be sorted alphabetically
    assert_eq!(loaded.records[0].tool_names, "Edit, Read");
}

// ---------------------------------------------------------------------------
// Per-line grouping
// ---------------------------------------------------------------------------

#[test]
fn test_per_line_grouping() {
    let records = vec![
        // Turn 1: user prompt → assistant response → tool result → assistant response
        mock_user_text("2026-01-15T10:00:00Z", "fix the bug"),
        mock_rec_with_tools(
            "claude-opus-4-6",
            1000,
            500,
            "2026-01-15T10:00:01Z",
            "r1",
            "m1",
            &["Read"],
        ),
        mock_user_tool_result("2026-01-15T10:00:02Z", "toolu_0000"),
        mock_rec(
            "claude-opus-4-6",
            2000,
            800,
            0,
            0,
            "2026-01-15T10:00:03Z",
            "r2",
            "m2",
        ),
        // Turn 2: user prompt → assistant response
        mock_user_text("2026-01-15T10:01:00Z", "now refactor it"),
        mock_rec_with_tools(
            "claude-opus-4-6",
            3000,
            1200,
            "2026-01-15T10:01:01Z",
            "r3",
            "m3",
            &["Edit"],
        ),
    ];

    let dir = make_fixture(&records);
    let opts = default_load_opts(dir.path());
    let loaded = load_records(&opts);
    // Only assistant messages with usage should be loaded (4 total, 2 user-text and 1 user-tool-result have no usage)
    assert_eq!(loaded.records.len(), 3);

    // Check line assignment — line is the record's own JSONL line number (1-based)
    // JSONL line 1: user text (no usage, not loaded)
    // JSONL line 2: assistant (usage) → line=2
    // JSONL line 3: user tool_result (no usage, not loaded)
    // JSONL line 4: assistant (usage) → line=4
    // JSONL line 5: user text (no usage, not loaded)
    // JSONL line 6: assistant (usage) → line=6
    assert_eq!(loaded.records[0].line, 2);
    assert_eq!(loaded.records[1].line, 4);
    assert_eq!(loaded.records[2].line, 6);

    let priced = calculate_cost(&loaded.records, None);
    let dims = vec![GroupDimension::Line];
    let grouped = group_records(&priced, &dims, Some(&default_group_opts()));

    // 3 records at lines 2, 4, 6 — each is its own group
    assert_eq!(grouped.data.len(), 3);
    let labels: Vec<&str> = grouped.data.iter().map(|g| g.label.as_str()).collect();
    assert!(labels.contains(&"#2"));
    assert!(labels.contains(&"#4"));
    assert!(labels.contains(&"#6"));

    let l2 = grouped.data.iter().find(|g| g.label == "#2").unwrap();
    assert_eq!(l2.input_tokens, 1000);

    let l6 = grouped.data.iter().find(|g| g.label == "#6").unwrap();
    assert_eq!(l6.input_tokens, 3000);
}

#[test]
fn test_per_session_per_tool_two_level() {
    let records = vec![
        mock_rec_with_tools(
            "claude-opus-4-6",
            1000,
            500,
            "2026-01-15T10:00:00Z",
            "r1",
            "m1",
            &["Read"],
        ),
        mock_rec_with_tools(
            "claude-opus-4-6",
            2000,
            800,
            "2026-01-15T10:01:00Z",
            "r2",
            "m2",
            &["Edit"],
        ),
    ];

    let dir = make_fixture(&records);
    let opts = default_load_opts(dir.path());
    let loaded = load_records(&opts);
    let priced = calculate_cost(&loaded.records, None);

    let dims = vec![GroupDimension::Session, GroupDimension::Tool];
    let grouped = group_records(&priced, &dims, Some(&default_group_opts()));

    // One session group
    assert_eq!(grouped.data.len(), 1);
    let children = grouped.data[0].children.as_ref().unwrap();
    assert_eq!(children.len(), 2);
    let child_labels: Vec<&str> = children.iter().map(|c| c.label.as_str()).collect();
    assert!(child_labels.contains(&"Edit"));
    assert!(child_labels.contains(&"Read"));
}
