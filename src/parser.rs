use chrono::{DateTime, Local, NaiveDateTime, TimeZone, Utc};
use rayon::prelude::*;
use serde::Deserialize;
use serde_json::Value;
use std::collections::{BTreeSet, HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use crate::types::{DedupStats, LoadOptions, LoadResult, RecordsMeta, TokenRecord};
use crate::utils::parse_fixed_offset;

/// Try to parse a single JSON value as a DateTime<Utc>.
/// - String: parse as ISO 8601
/// - Number > 1e12: milliseconds since epoch
/// - Number <= 1e12: seconds since epoch
fn parse_datetime_value(val: &Value) -> Option<DateTime<Utc>> {
    match val {
        Value::String(s) => {
            // Try parsing as RFC 3339 / ISO 8601
            if let Ok(dt) = DateTime::parse_from_rfc3339(s) {
                return Some(dt.with_timezone(&Utc));
            }
            // Try parsing without timezone (assume UTC)
            if let Ok(ndt) = NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S%.f") {
                return Some(ndt.and_utc());
            }
            if let Ok(ndt) = NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S") {
                return Some(ndt.and_utc());
            }
            // Try chrono's flexible parsing
            if let Ok(ndt) = s.parse::<DateTime<Utc>>() {
                return Some(ndt);
            }
            None
        }
        Value::Number(n) => {
            let num = n.as_f64()?;
            if num > 1e12 {
                // Milliseconds since epoch
                let secs = (num / 1000.0) as i64;
                let nsecs = ((num % 1000.0) * 1_000_000.0) as u32;
                Utc.timestamp_opt(secs, nsecs).single()
            } else {
                // Seconds since epoch
                let secs = num as i64;
                let nsecs = ((num - secs as f64) * 1_000_000_000.0) as u32;
                Utc.timestamp_opt(secs, nsecs).single()
            }
        }
        _ => None,
    }
}

/// Try to extract a timestamp from an object by probing the given field names in order.
fn probe_fields(obj: &Value, fields: &[&str]) -> Option<DateTime<Utc>> {
    for field in fields {
        if let Some(val) = obj.get(field) {
            if let Some(dt) = parse_datetime_value(val) {
                return Some(dt);
            }
        }
    }
    None
}

/// Extract a DateTime from a raw JSONL record by probing multiple fields and nested objects.
///
/// Search order:
/// 1. Top-level: timestamp, createdAt, updatedAt
/// 2. record.message: timestamp, createdAt
/// 3. record.snapshot: timestamp, createdAt
pub fn extract_timestamp(record: &Value) -> Option<DateTime<Utc>> {
    // 1. Top-level fields
    if let Some(dt) = probe_fields(record, &["timestamp", "createdAt", "updatedAt"]) {
        return Some(dt);
    }

    // 2. record.message fields (NOT updatedAt)
    if let Some(message) = record.get("message") {
        if let Some(dt) = probe_fields(message, &["timestamp", "createdAt"]) {
            return Some(dt);
        }
    }

    // 3. record.snapshot fields (NOT updatedAt)
    if let Some(snapshot) = record.get("snapshot") {
        if let Some(dt) = probe_fields(snapshot, &["timestamp", "createdAt"]) {
            return Some(dt);
        }
    }

    None
}

/// Navigate rec.message.usage.output_tokens, return 0 if not found.
fn get_output_tokens(rec: &Value) -> u64 {
    rec.get("message")
        .and_then(|m| m.get("usage"))
        .and_then(|u| u.get("output_tokens"))
        .and_then(|v| v.as_u64())
        .unwrap_or(0)
}

/// Index-based dedup: returns indices of records to keep (no cloning).
fn dedup_kept_indices(records: &[Value]) -> (Vec<usize>, DedupStats) {
    let before = records.len();
    let mut best: HashMap<String, (usize, u64)> = HashMap::new();
    let mut keyed_indices: HashSet<usize> = HashSet::new();

    for (i, rec) in records.iter().enumerate() {
        let message_id = rec
            .get("message")
            .and_then(|m| m.get("id"))
            .and_then(|v| v.as_str());
        let request_id = rec.get("requestId").and_then(|v| v.as_str());

        if let (Some(mid), Some(rid)) = (message_id, request_id) {
            let key = format!("{}:{}", mid, rid);
            let tokens = get_output_tokens(rec);
            keyed_indices.insert(i);

            match best.get(&key) {
                Some(&(_, existing_tokens)) if tokens < existing_tokens => {}
                _ => {
                    best.insert(key, (i, tokens));
                }
            }
        }
    }

    let winning_indices: HashSet<usize> = best.values().map(|&(idx, _)| idx).collect();

    let mut kept = Vec::with_capacity(before);
    for i in 0..before {
        if keyed_indices.contains(&i) {
            if winning_indices.contains(&i) {
                kept.push(i);
            }
        } else {
            kept.push(i);
        }
    }

    let after = kept.len();
    (kept, DedupStats { before, after })
}

/// Deduplicate streaming JSONL entries by keeping only the record with the highest
/// `output_tokens` for each unique `messageId:requestId` composite key.
///
/// Records without both `message.id` and `requestId` pass through unmodified.
pub fn deduplicate_streaming(records: &[Value]) -> (Vec<Value>, DedupStats) {
    let (kept_indices, stats) = dedup_kept_indices(records);
    let result = kept_indices
        .into_iter()
        .map(|i| records[i].clone())
        .collect();
    (result, stats)
}

/// Find the component after "projects" in the path. If it starts with `-`,
/// Extract sorted unique tool names from message content tool_use blocks.
fn extract_tool_names(msg: &RawMessage) -> String {
    let blocks = match &msg.content {
        Some(RawContent::Blocks(blocks)) => blocks,
        _ => return String::new(),
    };
    let mut names: Vec<&str> = blocks
        .iter()
        .filter(|item| item.item_type.as_deref() == Some("tool_use"))
        .filter_map(|item| item.name.as_deref())
        .collect();
    names.sort_unstable();
    names.dedup();
    names.join(", ")
}

/// replace all `-` with `/` to decode the path. Otherwise return as-is.
/// Return "unknown" if no "projects" segment.
pub fn extract_project_name(file_path: &str) -> String {
    let normalized = file_path.replace('\\', "/");
    let mut iter = normalized.split('/');
    while let Some(part) = iter.next() {
        if part == "projects" {
            if let Some(project_part) = iter.next() {
                if project_part.starts_with('-') {
                    return project_part.replacen('-', "/", project_part.len());
                } else {
                    return project_part.to_string();
                }
            }
        }
    }
    "unknown".to_string()
}

/// Format a DateTime as "YYYY-MM-DDTHH:MM:SS" in the specified timezone.
///
/// | tz value | Behavior |
/// |---|---|
/// | None, "local" | System local timezone |
/// | "UTC" | UTC formatting |
/// | "+HH:MM" / "-HH:MM" | Fixed UTC offset |
/// | IANA name | Use chrono_tz |
pub fn format_date_in_tz(date: &DateTime<Utc>, tz: Option<&str>) -> String {
    let format_str = "%Y-%m-%dT%H:%M:%S";

    match tz {
        None | Some("local") => {
            let local_dt = date.with_timezone(&Local);
            local_dt.format(format_str).to_string()
        }
        Some("UTC") => date.format(format_str).to_string(),
        Some(tz_str) => {
            // Try fixed offset: +HH:MM or -HH:MM
            if (tz_str.starts_with('+') || tz_str.starts_with('-')) && tz_str.len() == 6 {
                if let Some(offset) = parse_fixed_offset(tz_str) {
                    let dt = date.with_timezone(&offset);
                    return dt.format(format_str).to_string();
                }
            }

            // Try IANA timezone name
            if let Ok(tz_parsed) = tz_str.parse::<chrono_tz::Tz>() {
                let dt = date.with_timezone(&tz_parsed);
                return dt.format(format_str).to_string();
            }

            // Fallback to local
            let local_dt = date.with_timezone(&Local);
            local_dt.format(format_str).to_string()
        }
    }
}

/// Pre-resolved timezone for efficient repeated formatting in hot loops.
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

fn format_date_in_resolved_tz(date: &DateTime<Utc>, tz: &ResolvedTz) -> String {
    let format_str = "%Y-%m-%dT%H:%M:%S";
    match tz {
        ResolvedTz::Local => date.with_timezone(&Local).format(format_str).to_string(),
        ResolvedTz::Utc => date.format(format_str).to_string(),
        ResolvedTz::Fixed(offset) => date.with_timezone(offset).format(format_str).to_string(),
        ResolvedTz::Iana(tz) => date.with_timezone(tz).format(format_str).to_string(),
    }
}

// ─── Lightweight deserialization structs (skip message.content etc.) ───

#[derive(Deserialize)]
#[serde(untagged)]
enum TimestampVal {
    Str(String),
    Num(f64),
}

#[derive(Deserialize, Default)]
struct RawUsage {
    #[serde(default)]
    input_tokens: Option<u64>,
    #[serde(default)]
    output_tokens: Option<u64>,
    #[serde(default)]
    cache_creation_input_tokens: Option<u64>,
    #[serde(default)]
    cache_read_input_tokens: Option<u64>,
}

#[derive(Deserialize)]
struct RawContentItem {
    #[serde(default, rename = "type")]
    item_type: Option<String>,
    #[serde(default)]
    name: Option<String>,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum RawContent {
    Blocks(Vec<RawContentItem>),
    #[allow(dead_code)]
    Text(String),
}

impl Default for RawContent {
    fn default() -> Self {
        RawContent::Text(String::new())
    }
}

#[derive(Deserialize, Default)]
struct RawMessage {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    timestamp: Option<TimestampVal>,
    #[serde(default, rename = "createdAt")]
    created_at: Option<TimestampVal>,
    #[serde(default)]
    usage: Option<RawUsage>,
    #[serde(default)]
    content: Option<RawContent>,
}

#[derive(Deserialize, Default)]
struct RawTimestamps {
    #[serde(default)]
    timestamp: Option<TimestampVal>,
    #[serde(default, rename = "createdAt")]
    created_at: Option<TimestampVal>,
}

#[derive(Deserialize)]
struct RawRecord {
    #[serde(default, rename = "type")]
    record_type: Option<String>,
    #[serde(default, rename = "requestId")]
    request_id: Option<String>,
    #[serde(default)]
    timestamp: Option<TimestampVal>,
    #[serde(default, rename = "createdAt")]
    created_at: Option<TimestampVal>,
    #[serde(default, rename = "updatedAt")]
    updated_at: Option<TimestampVal>,
    #[serde(default)]
    message: Option<RawMessage>,
    #[serde(default)]
    snapshot: Option<RawTimestamps>,
}

fn parse_ts_val(val: &TimestampVal) -> Option<DateTime<Utc>> {
    match val {
        TimestampVal::Str(s) => {
            if let Ok(dt) = DateTime::parse_from_rfc3339(s) {
                return Some(dt.with_timezone(&Utc));
            }
            if let Ok(ndt) = NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S%.f") {
                return Some(ndt.and_utc());
            }
            if let Ok(ndt) = NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S") {
                return Some(ndt.and_utc());
            }
            s.parse::<DateTime<Utc>>().ok()
        }
        TimestampVal::Num(n) => {
            if *n > 1e12 {
                let secs = (*n / 1000.0) as i64;
                let nsecs = ((*n % 1000.0) * 1_000_000.0) as u32;
                Utc.timestamp_opt(secs, nsecs).single()
            } else {
                let secs = *n as i64;
                let nsecs = ((*n - secs as f64) * 1_000_000_000.0) as u32;
                Utc.timestamp_opt(secs, nsecs).single()
            }
        }
    }
}

fn extract_timestamp_raw(rec: &RawRecord) -> Option<DateTime<Utc>> {
    // Top-level: timestamp, createdAt, updatedAt
    for val in [&rec.timestamp, &rec.created_at, &rec.updated_at]
        .into_iter()
        .flatten()
    {
        if let Some(dt) = parse_ts_val(val) {
            return Some(dt);
        }
    }
    // message: timestamp, createdAt
    if let Some(ref msg) = rec.message {
        for val in [&msg.timestamp, &msg.created_at].into_iter().flatten() {
            if let Some(dt) = parse_ts_val(val) {
                return Some(dt);
            }
        }
    }
    // snapshot: timestamp, createdAt
    if let Some(ref snap) = rec.snapshot {
        for val in [&snap.timestamp, &snap.created_at].into_iter().flatten() {
            if let Some(dt) = parse_ts_val(val) {
                return Some(dt);
            }
        }
    }
    None
}

fn get_output_tokens_raw(rec: &RawRecord) -> u64 {
    rec.message
        .as_ref()
        .and_then(|m| m.usage.as_ref())
        .and_then(|u| u.output_tokens)
        .unwrap_or(0)
}

fn dedup_kept_indices_raw(records: &[RawRecord]) -> (Vec<usize>, DedupStats) {
    let before = records.len();
    let mut best: HashMap<String, (usize, u64)> = HashMap::new();
    let mut keyed_indices: HashSet<usize> = HashSet::new();

    for (i, rec) in records.iter().enumerate() {
        let message_id = rec.message.as_ref().and_then(|m| m.id.as_deref());
        let request_id = rec.request_id.as_deref();

        if let (Some(mid), Some(rid)) = (message_id, request_id) {
            let key = format!("{}:{}", mid, rid);
            let tokens = get_output_tokens_raw(rec);
            keyed_indices.insert(i);

            match best.get(&key) {
                Some(&(_, existing_tokens)) if tokens < existing_tokens => {}
                _ => {
                    best.insert(key, (i, tokens));
                }
            }
        }
    }

    let winning_indices: HashSet<usize> = best.values().map(|&(idx, _)| idx).collect();

    let mut kept = Vec::with_capacity(before);
    for i in 0..before {
        if keyed_indices.contains(&i) {
            if winning_indices.contains(&i) {
                kept.push(i);
            }
        } else {
            kept.push(i);
        }
    }

    let after = kept.len();
    (kept, DedupStats { before, after })
}

/// Extract session ID and agent ID from a JSONL file path.
///
/// Handles two subagent directory structures:
/// - Old: `<project>/subagents/<session>_<agent>.jsonl` → file stem as session, file stem as agent
/// - New: `<project>/<session-uuid>/subagents/<agent>.jsonl` → grandparent dir name as session, file stem as agent
///
/// For non-subagent files, uses the file stem (the session UUID) and empty agent_id.
///
/// Returns `(session_id, agent_id)`. `agent_id` is empty for main session files
/// and the subagent file stem for subagent files.
fn extract_session_and_agent(file_path: &Path) -> (String, String) {
    let file_stem = file_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_string();

    // Check if this file is under a `subagents/` directory
    if let Some(parent) = file_path.parent() {
        if parent.file_name().and_then(|n| n.to_str()) == Some("subagents") {
            if let Some(grandparent) = parent.parent() {
                if let Some(gp_name) = grandparent.file_name().and_then(|n| n.to_str()) {
                    // New structure: grandparent is a UUID-like session ID (not a project dir)
                    if gp_name.len() == 36 && gp_name.chars().filter(|&c| c == '-').count() == 4 {
                        return (gp_name.to_string(), file_stem);
                    }
                }
            }
            // Old structure: file is directly under <project>/subagents/
            return (file_stem.clone(), file_stem);
        }
    }
    // Default: main session file
    (file_stem, String::new())
}

/// Recursively find all *.jsonl files under a directory.
fn find_jsonl_files(dir: &Path) -> Vec<PathBuf> {
    let mut result = Vec::new();
    find_jsonl_files_recursive(dir, &mut result);
    result
}

fn find_jsonl_files_recursive(dir: &Path, result: &mut Vec<PathBuf>) {
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(_) => return,
    };

    for entry in entries {
        let entry = match entry {
            Ok(e) => e,
            Err(_) => continue,
        };
        let path = entry.path();
        if path.is_dir() {
            find_jsonl_files_recursive(&path, result);
        } else if let Some(ext) = path.extension() {
            if ext == "jsonl" {
                result.push(path);
            }
        }
    }
}

/// Return the first 10 characters (date portion) of a formatted datetime string.
fn get_date_part(formatted: &str) -> &str {
    if formatted.len() >= 10 {
        &formatted[..10]
    } else {
        formatted
    }
}

/// Main entry point. 6-step pipeline for loading and filtering token records.
pub fn load_records(options: &LoadOptions) -> LoadResult {
    // Step 1: Directory Discovery
    let project_dirs = discover_project_dirs(options);

    // Step 2: File Discovery
    let mut all_files: Vec<PathBuf> = Vec::new();
    for dir in &project_dirs {
        let mut files = find_jsonl_files(dir);
        all_files.append(&mut files);
    }

    // Step 3: Parallel parse & pre-filter using lightweight RawRecord (skips message.content)
    struct FileInfo {
        file_mtime: DateTime<Utc>,
        record_range: std::ops::Range<usize>,
        project: String,
        session_id: String,
        agent_id: String,
    }

    struct ParsedFile {
        file_mtime: DateTime<Utc>,
        records: Vec<RawRecord>,
        line_numbers: Vec<u32>,
        project: String,
        session_id: String,
        agent_id: String,
    }

    let parsed_files: Vec<ParsedFile> = all_files
        .par_iter()
        .filter_map(|file_path| {
            let content = fs::read_to_string(file_path).ok()?;
            let file_mtime = fs::metadata(file_path)
                .and_then(|m| m.modified())
                .map(DateTime::<Utc>::from)
                .unwrap_or_else(|_| Utc::now());

            let mut records = Vec::new();
            let mut line_numbers = Vec::new();
            let mut line_number: u32 = 0;

            for line in content.lines() {
                let line = line.trim();
                if line.is_empty() {
                    continue;
                }
                line_number += 1;

                let rec: RawRecord = match serde_json::from_str(line) {
                    Ok(r) => r,
                    Err(_) => continue,
                };

                // Type filter
                if let Some(ref t) = rec.record_type {
                    match t.as_str() {
                        "message" | "user" | "assistant" => {}
                        _ => continue,
                    }
                }

                // Must have usage with input_tokens and output_tokens
                let has_usage = rec
                    .message
                    .as_ref()
                    .and_then(|m| m.usage.as_ref())
                    .map(|u| u.input_tokens.is_some() && u.output_tokens.is_some())
                    .unwrap_or(false);
                if !has_usage {
                    continue;
                }

                // Must have model != "<synthetic>"
                match rec.message.as_ref().and_then(|m| m.model.as_deref()) {
                    Some(m) if m != "<synthetic>" => {}
                    _ => continue,
                }

                line_numbers.push(line_number);
                records.push(rec);
            }

            if records.is_empty() {
                return None;
            }

            let file_path_str = file_path.to_string_lossy();
            let project = extract_project_name(&file_path_str);
            let (session_id, agent_id) = extract_session_and_agent(file_path);

            Some(ParsedFile {
                file_mtime,
                records,
                line_numbers,
                project,
                session_id,
                agent_id,
            })
        })
        .collect();

    // Flatten into contiguous structure
    let mut all_records: Vec<RawRecord> = Vec::new();
    let mut all_line_numbers: Vec<u32> = Vec::new();
    let mut file_infos: Vec<FileInfo> = Vec::new();

    for pf in parsed_files {
        let start_idx = all_records.len();
        all_records.extend(pf.records);
        all_line_numbers.extend(pf.line_numbers);
        let end_idx = all_records.len();

        file_infos.push(FileInfo {
            file_mtime: pf.file_mtime,
            record_range: start_idx..end_idx,
            project: pf.project,
            session_id: pf.session_id,
            agent_id: pf.agent_id,
        });
    }

    // Step 4: Global Dedup (index-based, zero cloning)
    let (kept_indices, dedup_stats) = dedup_kept_indices_raw(&all_records);
    let kept_set: HashSet<usize> = kept_indices.iter().copied().collect();

    // Step 5: Timestamp Extraction (per-file, using indices into all_records)
    struct TimestampedRef {
        record_idx: usize,
        timestamp: DateTime<Utc>,
        file_info_idx: usize,
    }

    let mut timestamped: Vec<TimestampedRef> = Vec::with_capacity(kept_indices.len());

    for (fi_idx, fi) in file_infos.iter().enumerate() {
        let file_kept: Vec<usize> = fi
            .record_range
            .clone()
            .filter(|i| kept_set.contains(i))
            .collect();

        if file_kept.is_empty() {
            continue;
        }

        let first_ts_pos = file_kept
            .iter()
            .position(|&i| extract_timestamp_raw(&all_records[i]).is_some());

        match first_ts_pos {
            None => {
                for &idx in &file_kept {
                    timestamped.push(TimestampedRef {
                        record_idx: idx,
                        timestamp: fi.file_mtime,
                        file_info_idx: fi_idx,
                    });
                }
            }
            Some(first_pos) => {
                for &idx in &file_kept[..first_pos] {
                    timestamped.push(TimestampedRef {
                        record_idx: idx,
                        timestamp: fi.file_mtime,
                        file_info_idx: fi_idx,
                    });
                }
                for &idx in &file_kept[first_pos..] {
                    if let Some(ts) = extract_timestamp_raw(&all_records[idx]) {
                        timestamped.push(TimestampedRef {
                            record_idx: idx,
                            timestamp: ts,
                            file_info_idx: fi_idx,
                        });
                    }
                }
            }
        }
    }

    // Step 6: Filter & Build TokenRecords
    let needs_date_filter = options.from.is_some() || options.to.is_some();
    let resolved_tz = if needs_date_filter {
        Some(resolve_tz(options.tz.as_deref()))
    } else {
        None
    };

    let from_normalized = options.from.as_ref().map(|s| s.replace(' ', "T"));
    let to_normalized = options.to.as_ref().map(|s| s.replace(' ', "T"));
    let from_is_date_only = from_normalized
        .as_ref()
        .map(|s| s.len() == 10)
        .unwrap_or(false);
    let to_is_date_only = to_normalized
        .as_ref()
        .map(|s| s.len() == 10)
        .unwrap_or(false);

    // Pre-lowercase filter strings once
    let proj_filter_lower = options.project.as_ref().map(|s| s.to_lowercase());
    let model_filter_lower = options.model.as_ref().map(|s| s.to_lowercase());
    let session_filter_lower = options.session.as_ref().map(|s| s.to_lowercase());

    let mut filtered_records: Vec<TokenRecord> = Vec::new();

    // Single pass: build records AND collect meta
    let mut earliest: Option<DateTime<Utc>> = None;
    let mut latest: Option<DateTime<Utc>> = None;
    let mut projects_set = BTreeSet::new();
    let mut models_set = BTreeSet::new();
    let mut sessions_set = BTreeSet::new();

    for tr in &timestamped {
        let rec = &all_records[tr.record_idx];
        let fi = &file_infos[tr.file_info_idx];

        let model = rec
            .message
            .as_ref()
            .and_then(|m| m.model.as_deref())
            .unwrap_or("");

        // from/to filter (only format date if needed)
        if needs_date_filter {
            let formatted =
                format_date_in_resolved_tz(&tr.timestamp, resolved_tz.as_ref().unwrap());

            if let Some(ref from_val) = from_normalized {
                let cmp_value = if from_is_date_only {
                    get_date_part(&formatted)
                } else {
                    &formatted
                };
                if cmp_value < from_val.as_str() {
                    continue;
                }
            }

            if let Some(ref to_val) = to_normalized {
                let cmp_value = if to_is_date_only {
                    get_date_part(&formatted)
                } else {
                    &formatted
                };
                if cmp_value > to_val.as_str() {
                    continue;
                }
            }
        }

        // project filter
        if let Some(ref filter_lower) = proj_filter_lower {
            if !fi.project.to_lowercase().contains(filter_lower.as_str()) {
                continue;
            }
        }

        // model filter
        if let Some(ref filter_lower) = model_filter_lower {
            if !model.to_lowercase().contains(filter_lower.as_str()) {
                continue;
            }
        }

        // session filter
        if let Some(ref filter_lower) = session_filter_lower {
            if !fi.session_id.to_lowercase().contains(filter_lower.as_str()) {
                continue;
            }
        }

        // Extract token fields directly from struct
        let usage = rec.message.as_ref().and_then(|m| m.usage.as_ref());
        let input_tokens = usage.and_then(|u| u.input_tokens).unwrap_or(0);
        let output_tokens = usage.and_then(|u| u.output_tokens).unwrap_or(0);
        let cache_creation_tokens = usage
            .and_then(|u| u.cache_creation_input_tokens)
            .unwrap_or(0);
        let cache_read_tokens = usage.and_then(|u| u.cache_read_input_tokens).unwrap_or(0);

        // Update meta in same pass
        earliest = Some(match earliest {
            None => tr.timestamp,
            Some(e) => e.min(tr.timestamp),
        });
        latest = Some(match latest {
            None => tr.timestamp,
            Some(l) => l.max(tr.timestamp),
        });

        let model_owned = model.to_string();
        projects_set.insert(fi.project.clone());
        models_set.insert(model_owned.clone());
        sessions_set.insert(fi.session_id.clone());

        let tool_names = rec
            .message
            .as_ref()
            .map(extract_tool_names)
            .unwrap_or_default();

        filtered_records.push(TokenRecord {
            timestamp: tr.timestamp,
            model: model_owned,
            session_id: fi.session_id.clone(),
            project: fi.project.clone(),
            agent_id: fi.agent_id.clone(),
            tool_names,
            line: all_line_numbers[tr.record_idx],
            input_tokens,
            output_tokens,
            cache_creation_tokens,
            cache_read_tokens,
        });
    }

    let meta = RecordsMeta {
        earliest,
        latest,
        projects: projects_set.into_iter().collect(),
        models: models_set.into_iter().collect(),
        sessions: sessions_set.into_iter().collect(),
    };

    LoadResult {
        records: filtered_records,
        dedup: dedup_stats,
        meta,
    }
}

/// Discover project directories based on LoadOptions.
fn discover_project_dirs(options: &LoadOptions) -> Vec<PathBuf> {
    let mut candidate_dirs: Vec<PathBuf> = Vec::new();

    if let Some(ref claude_dir) = options.claude_dir {
        let projects_dir = PathBuf::from(claude_dir).join("projects");
        if projects_dir.is_dir() {
            candidate_dirs.push(projects_dir);
        }
    } else {
        // Default locations
        if let Some(home) = dirs::home_dir() {
            let dir1 = home.join(".claude").join("projects");
            if dir1.is_dir() {
                candidate_dirs.push(dir1);
            }
            let dir2 = home.join(".config").join("claude").join("projects");
            if dir2.is_dir() {
                candidate_dirs.push(dir2);
            }
        }
    }

    // Deduplicate via canonicalize
    let mut seen: HashSet<PathBuf> = HashSet::new();
    let mut unique_dirs: Vec<PathBuf> = Vec::new();

    for dir in candidate_dirs {
        if let Ok(canonical) = fs::canonicalize(&dir) {
            if seen.insert(canonical) {
                unique_dirs.push(dir);
            }
        } else {
            // If canonicalize fails, still include if not obviously duplicate
            unique_dirs.push(dir);
        }
    }

    unique_dirs
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn test_extract_project_name_encoded() {
        let path = "/home/user/.claude/projects/-home-user-workspace-test/sessions/abc.jsonl";
        assert_eq!(extract_project_name(path), "/home/user/workspace/test");
    }

    #[test]
    fn test_extract_project_name_plain() {
        let path = "/home/user/.claude/projects/myproject/sessions/abc.jsonl";
        assert_eq!(extract_project_name(path), "myproject");
    }

    #[test]
    fn test_extract_project_name_no_projects() {
        let path = "/home/user/.claude/sessions/abc.jsonl";
        assert_eq!(extract_project_name(path), "unknown");
    }

    #[test]
    fn test_extract_session_and_agent_main_file() {
        let path = Path::new(
            "/home/user/.claude/projects/-proj/abc12345-1234-5678-9abc-def012345678.jsonl",
        );
        let (session, agent) = extract_session_and_agent(path);
        assert_eq!(session, "abc12345-1234-5678-9abc-def012345678");
        assert_eq!(agent, "");
    }

    #[test]
    fn test_extract_session_and_agent_old_subagent() {
        // Old structure: projects/<project>/subagents/<session>_<agent>.jsonl
        let path = Path::new("/home/user/.claude/projects/-proj/subagents/abc12345_agent1.jsonl");
        // Parent is "subagents", grandparent is "-proj" (not a UUID) → falls back to file stem
        let (session, agent) = extract_session_and_agent(path);
        assert_eq!(session, "abc12345_agent1");
        assert_eq!(agent, "abc12345_agent1");
    }

    #[test]
    fn test_extract_session_and_agent_new_subagent() {
        // New structure: projects/<project>/<session-uuid>/subagents/<agent>.jsonl
        let path = Path::new("/home/user/.claude/projects/-proj/abc12345-1234-5678-9abc-def012345678/subagents/agent-a0f53f284339341b2.jsonl");
        // Parent is "subagents", grandparent is UUID → use grandparent
        let (session, agent) = extract_session_and_agent(path);
        assert_eq!(session, "abc12345-1234-5678-9abc-def012345678");
        assert_eq!(agent, "agent-a0f53f284339341b2");
    }

    #[test]
    fn test_format_date_utc() {
        let dt = Utc.with_ymd_and_hms(2025, 1, 15, 10, 30, 0).unwrap();
        assert_eq!(format_date_in_tz(&dt, Some("UTC")), "2025-01-15T10:30:00");
    }

    #[test]
    fn test_format_date_fixed_offset() {
        let dt = Utc.with_ymd_and_hms(2025, 1, 15, 10, 0, 0).unwrap();
        assert_eq!(
            format_date_in_tz(&dt, Some("+08:00")),
            "2025-01-15T18:00:00"
        );
    }

    #[test]
    fn test_format_date_iana() {
        let dt = Utc.with_ymd_and_hms(2025, 1, 15, 10, 0, 0).unwrap();
        let result = format_date_in_tz(&dt, Some("Asia/Shanghai"));
        assert_eq!(result, "2025-01-15T18:00:00");
    }

    #[test]
    fn test_extract_timestamp_top_level_string() {
        let record: Value =
            serde_json::from_str(r#"{"timestamp": "2025-01-15T10:30:00Z"}"#).unwrap();
        let ts = extract_timestamp(&record).unwrap();
        assert_eq!(ts, Utc.with_ymd_and_hms(2025, 1, 15, 10, 30, 0).unwrap());
    }

    #[test]
    fn test_extract_timestamp_millis() {
        // 1705314600000 ms = 2024-01-15T10:30:00Z
        let record: Value = serde_json::from_str(r#"{"timestamp": 1705314600000}"#).unwrap();
        let ts = extract_timestamp(&record).unwrap();
        assert_eq!(ts, Utc.with_ymd_and_hms(2024, 1, 15, 10, 30, 0).unwrap());
    }

    #[test]
    fn test_extract_timestamp_seconds() {
        let record: Value = serde_json::from_str(r#"{"timestamp": 1705314600}"#).unwrap();
        let ts = extract_timestamp(&record).unwrap();
        assert_eq!(ts, Utc.with_ymd_and_hms(2024, 1, 15, 10, 30, 0).unwrap());
    }

    #[test]
    fn test_extract_timestamp_nested_message() {
        let record: Value =
            serde_json::from_str(r#"{"message": {"createdAt": "2025-01-15T10:30:00Z"}}"#).unwrap();
        let ts = extract_timestamp(&record).unwrap();
        assert_eq!(ts, Utc.with_ymd_and_hms(2025, 1, 15, 10, 30, 0).unwrap());
    }

    #[test]
    fn test_extract_timestamp_none() {
        let record: Value = serde_json::from_str(r#"{"foo": "bar"}"#).unwrap();
        assert!(extract_timestamp(&record).is_none());
    }

    #[test]
    fn test_deduplicate_streaming() {
        let records: Vec<Value> = vec![
            serde_json::from_str(
                r#"{"message": {"id": "m1", "usage": {"output_tokens": 100}}, "requestId": "r1"}"#,
            )
            .unwrap(),
            serde_json::from_str(
                r#"{"message": {"id": "m1", "usage": {"output_tokens": 200}}, "requestId": "r1"}"#,
            )
            .unwrap(),
            serde_json::from_str(r#"{"message": {"id": "m2", "usage": {"output_tokens": 50}}}"#)
                .unwrap(),
        ];

        let (deduped, stats) = deduplicate_streaming(&records);
        assert_eq!(stats.before, 3);
        assert_eq!(stats.after, 2);
        // The second record (200 tokens) should win for m1:r1
        assert_eq!(get_output_tokens(&deduped[0]), 200);
        // The record without requestId passes through
        assert_eq!(get_output_tokens(&deduped[1]), 50);
    }

    #[test]
    fn test_get_date_part() {
        assert_eq!(get_date_part("2025-01-15T10:30:00"), "2025-01-15");
        assert_eq!(get_date_part("short"), "short");
    }

    // ─── extract_timestamp – additional cases ────────────────────────────────

    #[test]
    fn test_extract_timestamp_top_level_numeric_millis() {
        // Numeric timestamp > 1e12 interpreted as milliseconds
        // 1705314600000 ms = 2024-01-15T10:30:00Z
        let record: Value = serde_json::from_str(r#"{"createdAt": 1705314600000}"#).unwrap();
        let ts = extract_timestamp(&record).unwrap();
        assert_eq!(ts, Utc.with_ymd_and_hms(2024, 1, 15, 10, 30, 0).unwrap());
    }

    #[test]
    fn test_extract_timestamp_top_level_numeric_seconds() {
        // Numeric timestamp <= 1e12 interpreted as seconds
        let record: Value = serde_json::from_str(r#"{"timestamp": 1705314600}"#).unwrap();
        let ts = extract_timestamp(&record).unwrap();
        assert_eq!(ts, Utc.with_ymd_and_hms(2024, 1, 15, 10, 30, 0).unwrap());
    }

    #[test]
    fn test_extract_timestamp_snapshot_field() {
        // Timestamp nested inside "snapshot" object
        let record: Value =
            serde_json::from_str(r#"{"snapshot": {"createdAt": "2025-03-20T12:00:00Z"}}"#).unwrap();
        let ts = extract_timestamp(&record).unwrap();
        assert_eq!(ts, Utc.with_ymd_and_hms(2025, 3, 20, 12, 0, 0).unwrap());
    }

    #[test]
    fn test_extract_timestamp_prefers_top_level_over_message() {
        // When both top-level and message.createdAt exist, top-level wins
        let record: Value = serde_json::from_str(
            r#"{
                "timestamp": "2025-01-15T10:30:00Z",
                "message": {"createdAt": "2025-06-01T00:00:00Z"}
            }"#,
        )
        .unwrap();
        let ts = extract_timestamp(&record).unwrap();
        assert_eq!(ts, Utc.with_ymd_and_hms(2025, 1, 15, 10, 30, 0).unwrap());
    }

    // ─── deduplicate_streaming – additional cases ────────────────────────────

    #[test]
    fn test_deduplicate_keeps_higher_output() {
        // Two records with same messageId:requestId but different output_tokens → keep higher
        let records: Vec<Value> = vec![
            serde_json::from_str(
                r#"{"message": {"id": "msgX", "usage": {"output_tokens": 50}}, "requestId": "reqX"}"#,
            )
            .unwrap(),
            serde_json::from_str(
                r#"{"message": {"id": "msgX", "usage": {"output_tokens": 300}}, "requestId": "reqX"}"#,
            )
            .unwrap(),
        ];

        let (deduped, stats) = deduplicate_streaming(&records);
        assert_eq!(stats.before, 2);
        assert_eq!(stats.after, 1);
        assert_eq!(get_output_tokens(&deduped[0]), 300);
    }

    #[test]
    fn test_deduplicate_empty_input() {
        let records: Vec<Value> = vec![];
        let (deduped, stats) = deduplicate_streaming(&records);
        assert!(deduped.is_empty());
        assert_eq!(stats.before, 0);
        assert_eq!(stats.after, 0);
    }

    #[test]
    fn test_deduplicate_no_keyed_records_pass_through() {
        // Records without message.id or requestId are not subject to dedup
        let records: Vec<Value> = vec![
            serde_json::from_str(r#"{"foo": "bar"}"#).unwrap(),
            serde_json::from_str(r#"{"baz": 42}"#).unwrap(),
        ];
        let (deduped, stats) = deduplicate_streaming(&records);
        assert_eq!(stats.before, 2);
        assert_eq!(stats.after, 2);
        assert_eq!(deduped.len(), 2);
    }

    // ─── extract_project_name – additional cases ─────────────────────────────

    #[test]
    fn test_extract_project_name_deeply_nested() {
        // Path with many components after "projects"
        let path =
            "/home/user/.claude/projects/-home-user-deep-path-to-workspace/session/sub/abc.jsonl";
        assert_eq!(
            extract_project_name(path),
            "/home/user/deep/path/to/workspace"
        );
    }

    #[test]
    fn test_extract_project_name_single_component() {
        // project part does not start with '-', returned as-is
        let path = "/projects/test";
        assert_eq!(extract_project_name(path), "test");
    }

    #[test]
    fn test_extract_project_name_projects_at_root() {
        // "projects" is the first segment; next part starts with '-'
        let path = "projects/-home-alice-code";
        assert_eq!(extract_project_name(path), "/home/alice/code");
    }

    // ─── format_date_in_tz – local timezone fallback ─────────────────────────

    #[test]
    fn test_format_date_local_returns_string() {
        // With tz=None, output should be a non-empty datetime string (content varies by machine TZ)
        let dt = Utc.with_ymd_and_hms(2025, 6, 15, 12, 0, 0).unwrap();
        let result = format_date_in_tz(&dt, None);
        // Should look like "YYYY-MM-DDTHH:MM:SS" (19 chars)
        assert_eq!(result.len(), 19, "unexpected length: {result}");
        assert!(result.contains('-') && result.contains('T') && result.contains(':'));
    }

    #[test]
    fn test_format_date_local_string_explicit() {
        // With tz=Some("local"), should behave the same as None
        let dt = Utc.with_ymd_and_hms(2025, 6, 15, 12, 0, 0).unwrap();
        let result_none = format_date_in_tz(&dt, None);
        let result_local = format_date_in_tz(&dt, Some("local"));
        assert_eq!(result_none, result_local);
    }

    #[test]
    fn test_format_date_invalid_tz_falls_back_to_local() {
        // An unrecognised timezone string falls back to local
        let dt = Utc.with_ymd_and_hms(2025, 6, 15, 12, 0, 0).unwrap();
        let result_local = format_date_in_tz(&dt, None);
        let result_invalid = format_date_in_tz(&dt, Some("Not/AReal/Timezone"));
        assert_eq!(result_local, result_invalid);
    }
}
