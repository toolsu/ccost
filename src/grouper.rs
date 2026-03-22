use std::collections::HashMap;
use std::sync::LazyLock;

use regex::Regex;

use crate::types::{
    GroupDimension, GroupOptions, GroupResult, GroupedData, PricedTokenRecord, SortOrder,
};

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
        GroupDimension::Day => format_timestamp_resolved(record, "%Y-%m-%d", tz),
        GroupDimension::Hour => format_timestamp_resolved(record, "%Y-%m-%d %H:00", tz),
        GroupDimension::Month => format_timestamp_resolved(record, "%Y-%m", tz),
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

/// Parse a fixed offset string like `"+05:30"` or `"-08:00"` into a `FixedOffset`.
fn parse_fixed_offset(s: &str) -> Option<chrono::FixedOffset> {
    if s.len() < 5 {
        return None;
    }
    let sign = match s.as_bytes()[0] {
        b'+' => 1,
        b'-' => -1,
        _ => return None,
    };
    let rest = &s[1..];
    let parts: Vec<&str> = rest.split(':').collect();
    if parts.len() != 2 {
        return None;
    }
    let hours: i32 = parts[0].parse().ok()?;
    let minutes: i32 = parts[1].parse().ok()?;
    let total_seconds = sign * (hours * 3600 + minutes * 60);
    chrono::FixedOffset::east_opt(total_seconds)
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
fn sort_grouped_data(data: &mut [GroupedData], order: SortOrder) {
    match order {
        SortOrder::Asc => data.sort_by(|a, b| a.label.cmp(&b.label)),
        SortOrder::Desc => data.sort_by(|a, b| b.label.cmp(&a.label)),
    }
}
