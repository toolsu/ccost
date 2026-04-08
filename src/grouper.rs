use std::collections::HashMap;
use std::sync::LazyLock;

use regex::Regex;

use crate::types::{
    GroupDimension, GroupOptions, GroupResult, GroupedData, PricedTokenRecord, SortOrder,
};
use crate::utils::parse_fixed_offset;

static DATE_SUFFIX_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"-\d{8}$").unwrap());

/// Shorten a model name by stripping the `claude-` prefix and any trailing
/// `-YYYYMMDD` date suffix (8 digits after a dash).
///
/// Example: `"claude-opus-4-6-20250618"` becomes `"opus-4-6"`.
pub fn shorten_model_name(model: &str) -> String {
    let s = model.strip_prefix("claude-").unwrap_or(model);
    DATE_SUFFIX_RE.replace(s, "").to_string()
}

/// Pre-resolved timezone for efficient repeated formatting.
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
            if let Some(fixed) = parse_fixed_offset(s) {
                ResolvedTz::Fixed(fixed)
            } else if let Ok(iana) = s.parse::<chrono_tz::Tz>() {
                ResolvedTz::Iana(iana)
            } else {
                ResolvedTz::Local
            }
        }
    }
}

/// Produce a group key for the given record according to the specified dimension.
///
/// Time-based dimensions format the record's timestamp in the requested timezone.
/// Non-time dimensions return the raw field value (or shortened model name).
pub fn get_group_key(
    record: &PricedTokenRecord,
    dimension: GroupDimension,
    tz: Option<&str>,
) -> String {
    let resolved = resolve_tz(tz);
    get_group_key_resolved(record, dimension, &resolved)
}

fn get_group_key_resolved(
    record: &PricedTokenRecord,
    dimension: GroupDimension,
    tz: &ResolvedTz,
) -> String {
    match dimension {
        GroupDimension::Session => record.session_id.clone(),
        GroupDimension::Project => record.project.clone(),
        GroupDimension::Model => shorten_model_name(&record.model),
        GroupDimension::Subagent => {
            if record.agent_id.is_empty() {
                "(main)".to_string()
            } else {
                record.agent_id.clone()
            }
        }
        GroupDimension::Day => format_timestamp_resolved(record, "%Y-%m-%d", tz),
        GroupDimension::Hour => format_timestamp_resolved(record, "%Y-%m-%d %H:00", tz),
        GroupDimension::Month => format_timestamp_resolved(record, "%Y-%m", tz),
        GroupDimension::Tool => {
            if record.tool_names.is_empty() {
                "(text)".to_string()
            } else {
                record.tool_names.clone()
            }
        }
        GroupDimension::Line => format!("#{}", record.line),
    }
}

fn format_timestamp_resolved(record: &PricedTokenRecord, fmt: &str, tz: &ResolvedTz) -> String {
    match tz {
        ResolvedTz::Local => record
            .timestamp
            .with_timezone(&chrono::Local)
            .format(fmt)
            .to_string(),
        ResolvedTz::Utc => record.timestamp.format(fmt).to_string(),
        ResolvedTz::Fixed(offset) => record
            .timestamp
            .with_timezone(offset)
            .format(fmt)
            .to_string(),
        ResolvedTz::Iana(iana_tz) => record
            .timestamp
            .with_timezone(iana_tz)
            .format(fmt)
            .to_string(),
    }
}

/// Aggregate numeric fields from a slice of priced token records into a `GroupedData`.
fn aggregate(label: &str, records: &[&PricedTokenRecord]) -> GroupedData {
    let mut input_tokens: u64 = 0;
    let mut output_tokens: u64 = 0;
    let mut cache_creation_tokens: u64 = 0;
    let mut cache_read_tokens: u64 = 0;
    let mut input_cost: f64 = 0.0;
    let mut cache_creation_cost: f64 = 0.0;
    let mut cache_read_cost: f64 = 0.0;
    let mut output_cost: f64 = 0.0;
    let mut total_cost: f64 = 0.0;

    for r in records {
        input_tokens += r.input_tokens;
        output_tokens += r.output_tokens;
        cache_creation_tokens += r.cache_creation_tokens;
        cache_read_tokens += r.cache_read_tokens;
        input_cost += r.input_cost;
        cache_creation_cost += r.cache_creation_cost;
        cache_read_cost += r.cache_read_cost;
        output_cost += r.output_cost;
        total_cost += r.total_cost;
    }

    GroupedData {
        label: label.to_string(),
        input_tokens,
        output_tokens,
        cache_creation_tokens,
        cache_read_tokens,
        input_cost,
        cache_creation_cost,
        cache_read_cost,
        output_cost,
        total_cost,
        children: None,
    }
}

/// Group priced token records by one or two dimensions, producing sorted
/// aggregated data and grand totals.
pub fn group_records(
    records: &[PricedTokenRecord],
    dimensions: &[GroupDimension],
    options: Option<&GroupOptions>,
) -> GroupResult {
    let default_opts = GroupOptions::default();
    let opts = options.unwrap_or(&default_opts);
    let resolved = resolve_tz(opts.tz.as_deref());

    // Grand totals across ALL records
    let all_refs: Vec<&PricedTokenRecord> = records.iter().collect();
    let mut totals = aggregate("TOTAL", &all_refs);

    // Empty input or no dimensions
    if records.is_empty() || dimensions.is_empty() {
        return GroupResult {
            data: vec![],
            totals,
        };
    }

    let dim1 = dimensions[0];
    let dim2 = dimensions.get(1).copied();

    match dim2 {
        None => {
            // Single dimension grouping
            let mut buckets: HashMap<String, Vec<&PricedTokenRecord>> = HashMap::new();
            for record in records {
                let key = get_group_key_resolved(record, dim1, &resolved);
                buckets.entry(key).or_default().push(record);
            }

            let mut data: Vec<GroupedData> = buckets
                .iter()
                .map(|(key, recs)| aggregate(key, recs))
                .collect();

            sort_grouped_data(&mut data, opts.order);

            GroupResult { data, totals }
        }
        Some(dim2) => {
            // Two dimension grouping
            // Bucket by dim1, then within each by dim2
            let mut parent_buckets: HashMap<String, HashMap<String, Vec<&PricedTokenRecord>>> =
                HashMap::new();

            // Also track all unique dim2 keys globally for grand total children
            let mut global_dim2: HashMap<String, Vec<&PricedTokenRecord>> = HashMap::new();

            for record in records {
                let key1 = get_group_key_resolved(record, dim1, &resolved);
                let key2 = get_group_key_resolved(record, dim2, &resolved);

                parent_buckets
                    .entry(key1)
                    .or_default()
                    .entry(key2.clone())
                    .or_default()
                    .push(record);

                global_dim2.entry(key2).or_default().push(record);
            }

            let mut data: Vec<GroupedData> = parent_buckets
                .iter()
                .map(|(parent_key, child_buckets)| {
                    // Build children
                    let mut children: Vec<GroupedData> = child_buckets
                        .iter()
                        .map(|(child_key, recs)| aggregate(child_key, recs))
                        .collect();

                    sort_grouped_data(&mut children, opts.order);

                    // Parent = sum of all children's records
                    let all_child_recs: Vec<&PricedTokenRecord> = child_buckets
                        .values()
                        .flat_map(|v| v.iter().copied())
                        .collect();
                    let mut parent = aggregate(parent_key, &all_child_recs);
                    parent.children = Some(children);
                    parent
                })
                .collect();

            sort_grouped_data(&mut data, opts.order);

            // Grand total children: one per unique dim2 value
            let mut total_children: Vec<GroupedData> = global_dim2
                .iter()
                .map(|(key, recs)| aggregate(key, recs))
                .collect();
            sort_grouped_data(&mut total_children, opts.order);
            totals.children = Some(total_children);

            GroupResult { data, totals }
        }
    }
}

/// Sort a vec of GroupedData by label, in the specified order.
/// Extract numeric value from a "#N" or "#N (setup)" label for sorting.
fn turn_sort_key(label: &str) -> Option<u32> {
    let s = label.strip_prefix('#')?;
    let num_part = s.split_once(' ').map(|(n, _)| n).unwrap_or(s);
    num_part.parse().ok()
}

fn sort_grouped_data(data: &mut [GroupedData], order: SortOrder) {
    match order {
        SortOrder::Asc => {
            data.sort_by(
                |a, b| match (turn_sort_key(&a.label), turn_sort_key(&b.label)) {
                    (Some(na), Some(nb)) => na.cmp(&nb),
                    _ => a.label.cmp(&b.label),
                },
            )
        }
        SortOrder::Desc => {
            data.sort_by(
                |a, b| match (turn_sort_key(&a.label), turn_sort_key(&b.label)) {
                    (Some(na), Some(nb)) => nb.cmp(&na),
                    _ => b.label.cmp(&a.label),
                },
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{TimeZone, Utc};

    fn mock_record(date: &str, model: &str, session: &str, project: &str) -> PricedTokenRecord {
        let ts = Utc
            .with_ymd_and_hms(
                date[..4].parse().unwrap(),
                date[5..7].parse().unwrap(),
                date[8..10].parse().unwrap(),
                12,
                0,
                0,
            )
            .unwrap();
        PricedTokenRecord {
            timestamp: ts,
            model: model.to_string(),
            session_id: session.to_string(),
            project: project.to_string(),
            agent_id: String::new(),
            tool_names: String::new(),
            line: 0,
            input_tokens: 100,
            output_tokens: 50,
            cache_creation_tokens: 0,
            cache_read_tokens: 0,
            input_cost: 0.01,
            cache_creation_cost: 0.0,
            cache_read_cost: 0.0,
            output_cost: 0.02,
            total_cost: 0.03,
        }
    }

    #[test]
    fn test_shorten_model_name_strips_claude_and_date() {
        assert_eq!(
            shorten_model_name("claude-3-5-sonnet-20241022"),
            "3-5-sonnet"
        );
    }

    #[test]
    fn test_shorten_model_name_no_prefix() {
        assert_eq!(shorten_model_name("gpt-4"), "gpt-4");
    }

    #[test]
    fn test_shorten_model_name_claude_no_date() {
        assert_eq!(shorten_model_name("claude-opus"), "opus");
    }

    #[test]
    fn test_get_group_key_session() {
        let rec = mock_record("2026-03-15", "model", "sess-abc", "proj");
        assert_eq!(
            get_group_key(&rec, GroupDimension::Session, None),
            "sess-abc"
        );
    }

    #[test]
    fn test_get_group_key_project() {
        let rec = mock_record("2026-03-15", "model", "s1", "my-project");
        assert_eq!(
            get_group_key(&rec, GroupDimension::Project, None),
            "my-project"
        );
    }

    #[test]
    fn test_get_group_key_model_shortens() {
        let rec = mock_record("2026-03-15", "claude-3-5-sonnet-20241022", "s1", "proj");
        assert_eq!(
            get_group_key(&rec, GroupDimension::Model, None),
            "3-5-sonnet"
        );
    }

    #[test]
    fn test_get_group_key_day_utc() {
        let rec = mock_record("2026-03-15", "model", "s1", "proj");
        let key = get_group_key(&rec, GroupDimension::Day, Some("UTC"));
        assert_eq!(key, "2026-03-15");
    }

    #[test]
    fn test_get_group_key_hour_utc() {
        let rec = mock_record("2026-03-15", "model", "s1", "proj");
        let key = get_group_key(&rec, GroupDimension::Hour, Some("UTC"));
        assert_eq!(key, "2026-03-15 12:00");
    }

    #[test]
    fn test_get_group_key_month_utc() {
        let rec = mock_record("2026-03-15", "model", "s1", "proj");
        let key = get_group_key(&rec, GroupDimension::Month, Some("UTC"));
        assert_eq!(key, "2026-03");
    }

    #[test]
    fn test_get_group_key_subagent_main_session() {
        let rec = mock_record("2026-03-15", "model", "s1", "proj");
        // agent_id is empty for main session → "(main)"
        assert_eq!(
            get_group_key(&rec, GroupDimension::Subagent, None),
            "(main)"
        );
    }

    #[test]
    fn test_get_group_key_subagent_with_agent_id() {
        let mut rec = mock_record("2026-03-15", "model", "s1", "proj");
        rec.agent_id = "agent-a0f53f284339341b2".to_string();
        assert_eq!(
            get_group_key(&rec, GroupDimension::Subagent, None),
            "agent-a0f53f284339341b2"
        );
    }

    #[test]
    fn test_group_by_subagent() {
        let main_rec = mock_record("2026-03-15", "model", "s1", "proj");
        let mut sub_rec = mock_record("2026-03-15", "model", "s1", "proj");
        sub_rec.agent_id = "agent-abc".to_string();

        // Double the sub_rec tokens to distinguish
        sub_rec.input_tokens = 200;
        sub_rec.output_tokens = 100;

        let records = vec![main_rec, sub_rec];
        let grouped = group_records(
            &records,
            &[GroupDimension::Subagent],
            Some(&GroupOptions {
                order: SortOrder::Asc,
                tz: None,
            }),
        );

        assert_eq!(grouped.data.len(), 2);
        let labels: Vec<&str> = grouped.data.iter().map(|d| d.label.as_str()).collect();
        assert!(labels.contains(&"(main)"));
        assert!(labels.contains(&"agent-abc"));

        let main_group = grouped.data.iter().find(|d| d.label == "(main)").unwrap();
        assert_eq!(main_group.input_tokens, 100);

        let sub_group = grouped
            .data
            .iter()
            .find(|d| d.label == "agent-abc")
            .unwrap();
        assert_eq!(sub_group.input_tokens, 200);
    }

    #[test]
    fn test_resolve_tz_utc() {
        let tz = resolve_tz(Some("UTC"));
        assert!(matches!(tz, ResolvedTz::Utc));
    }

    #[test]
    fn test_resolve_tz_fixed() {
        let tz = resolve_tz(Some("+05:30"));
        assert!(matches!(tz, ResolvedTz::Fixed(_)));
    }

    #[test]
    fn test_resolve_tz_iana() {
        let tz = resolve_tz(Some("America/New_York"));
        assert!(matches!(tz, ResolvedTz::Iana(_)));
    }

    #[test]
    fn test_resolve_tz_none_is_local() {
        let tz = resolve_tz(None);
        assert!(matches!(tz, ResolvedTz::Local));
    }
}
