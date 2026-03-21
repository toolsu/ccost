use crate::types::{GroupedData, PriceMode};
use super::table::{format_tokens, format_cost};

pub struct DsvOptions {
    pub dimension_label: String,
    pub price_mode: PriceMode,
    pub compact: bool,
}

/// Format a cell value: tokens with optional cost.
fn format_cell(tokens: u64, cost: f64, price_mode: PriceMode) -> String {
    let token_str = format_tokens(tokens);
    if price_mode == PriceMode::Off {
        token_str
    } else {
        format!("{} ({})", token_str, format_cost(cost, price_mode))
    }
}

struct RowData {
    label: String,
    cells: Vec<String>,
}

fn build_row(entry: &GroupedData, price_mode: PriceMode, compact: bool, label_prefix: &str) -> RowData {
    let in_total = entry.input_tokens + entry.cache_creation_tokens + entry.cache_read_tokens;
    let in_total_cost = entry.input_cost + entry.cache_creation_cost + entry.cache_read_cost;
    let total = in_total + entry.output_tokens;
    let total_cost = entry.total_cost;

    let label = format!("{}{}", label_prefix, entry.label);

    let cells = if compact {
        vec![
            format_cell(in_total, in_total_cost, price_mode),
            format_cell(entry.output_tokens, entry.output_cost, price_mode),
            format_cell(total, total_cost, price_mode),
        ]
    } else {
        vec![
            format_cell(entry.input_tokens, entry.input_cost, price_mode),
            format_cell(entry.cache_creation_tokens, entry.cache_creation_cost, price_mode),
            format_cell(entry.cache_read_tokens, entry.cache_read_cost, price_mode),
            format_cell(in_total, in_total_cost, price_mode),
            format_cell(entry.output_tokens, entry.output_cost, price_mode),
            format_cell(total, total_cost, price_mode),
        ]
    };

    RowData { label, cells }
}

fn collect_all_rows(data: &[GroupedData], totals: &GroupedData, price_mode: PriceMode, compact: bool) -> (Vec<String>, Vec<RowData>) {
    let headers: Vec<String> = if compact {
        vec![
            "In Total".to_string(),
            "Out".to_string(),
            "Total".to_string(),
        ]
    } else {
        vec![
            "In".to_string(),
            "Cache Cr".to_string(),
            "Cache Rd".to_string(),
            "In Total".to_string(),
            "Out".to_string(),
            "Total".to_string(),
        ]
    };

    let mut rows = Vec::new();

    // Data rows
    for entry in data {
        rows.push(build_row(entry, price_mode, compact, ""));
        if let Some(ref children) = entry.children {
            for child in children {
                rows.push(build_row(child, price_mode, compact, "\u{2514}\u{2500} "));
            }
        }
    }

    // Totals row
    let mut totals_row = build_row(totals, price_mode, compact, "");
    totals_row.label = "TOTAL".to_string();
    rows.push(totals_row);

    // Totals children
    if let Some(ref children) = totals.children {
        for child in children {
            rows.push(build_row(child, price_mode, compact, "\u{2514}\u{2500} "));
        }
    }

    (headers, rows)
}

/// Escape a field for CSV (RFC 4180).
/// Quote fields containing comma, double-quote, newline, or carriage return.
/// Double any quotes inside quoted fields.
fn csv_escape(field: &str) -> String {
    if field.contains(',') || field.contains('"') || field.contains('\n') || field.contains('\r') {
        let escaped = field.replace('"', "\"\"");
        format!("\"{}\"", escaped)
    } else {
        field.to_string()
    }
}

/// Escape a field for TSV using backslash escaping.
fn tsv_escape(field: &str) -> String {
    field
        .replace('\\', "\\\\")
        .replace('\t', "\\t")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
}

/// Format data as CSV with comma delimiter and RFC 4180 escaping.
pub fn format_csv(
    data: &[GroupedData],
    totals: &GroupedData,
    options: &DsvOptions,
) -> String {
    let (headers, rows) = collect_all_rows(data, totals, options.price_mode, options.compact);

    let mut output = String::new();

    // Header row
    let mut header_fields = vec![csv_escape(&options.dimension_label)];
    for h in &headers {
        header_fields.push(csv_escape(h));
    }
    output.push_str(&header_fields.join(","));
    output.push('\n');

    // Data rows
    for row in &rows {
        let mut fields = vec![csv_escape(&row.label)];
        for cell in &row.cells {
            fields.push(csv_escape(cell));
        }
        output.push_str(&fields.join(","));
        output.push('\n');
    }

    output
}

/// Format data as TSV with tab delimiter and backslash escaping.
pub fn format_tsv(
    data: &[GroupedData],
    totals: &GroupedData,
    options: &DsvOptions,
) -> String {
    let (headers, rows) = collect_all_rows(data, totals, options.price_mode, options.compact);

    let mut output = String::new();

    // Header row
    let mut header_fields = vec![tsv_escape(&options.dimension_label)];
    for h in &headers {
        header_fields.push(tsv_escape(h));
    }
    output.push_str(&header_fields.join("\t"));
    output.push('\n');

    // Data rows
    for row in &rows {
        let mut fields = vec![tsv_escape(&row.label)];
        for cell in &row.cells {
            fields.push(tsv_escape(cell));
        }
        output.push_str(&fields.join("\t"));
        output.push('\n');
    }

    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_csv_escape() {
        assert_eq!(csv_escape("hello"), "hello");
        assert_eq!(csv_escape("hello,world"), "\"hello,world\"");
        assert_eq!(csv_escape("say \"hi\""), "\"say \"\"hi\"\"\"");
        assert_eq!(csv_escape("line\nbreak"), "\"line\nbreak\"");
    }

    #[test]
    fn test_tsv_escape() {
        assert_eq!(tsv_escape("hello"), "hello");
        assert_eq!(tsv_escape("tab\there"), "tab\\there");
        assert_eq!(tsv_escape("back\\slash"), "back\\\\slash");
        assert_eq!(tsv_escape("new\nline"), "new\\nline");
        assert_eq!(tsv_escape("cr\rreturn"), "cr\\rreturn");
    }

    #[test]
    fn test_format_csv_basic() {
        let data = vec![GroupedData {
            label: "2025-01".to_string(),
            input_tokens: 1000,
            output_tokens: 500,
            cache_creation_tokens: 200,
            cache_read_tokens: 300,
            input_cost: 0.10,
            cache_creation_cost: 0.02,
            cache_read_cost: 0.03,
            output_cost: 0.05,
            total_cost: 0.20,
            children: None,
        }];
        let totals = data[0].clone();

        let options = DsvOptions {
            dimension_label: "Month".to_string(),
            price_mode: PriceMode::Off,
            compact: false,
        };

        let result = format_csv(&data, &totals, &options);
        let lines: Vec<&str> = result.lines().collect();
        assert_eq!(lines[0], "Month,In,Cache Cr,Cache Rd,In Total,Out,Total");
        assert!(result.contains("2025-01"));
        assert!(result.contains("TOTAL"));
        assert!(result.ends_with('\n'));
    }

    #[test]
    fn test_format_tsv_basic() {
        let data = vec![GroupedData {
            label: "2025-01".to_string(),
            input_tokens: 1000,
            output_tokens: 500,
            cache_creation_tokens: 0,
            cache_read_tokens: 0,
            input_cost: 0.0,
            cache_creation_cost: 0.0,
            cache_read_cost: 0.0,
            output_cost: 0.0,
            total_cost: 0.0,
            children: None,
        }];
        let totals = data[0].clone();

        let options = DsvOptions {
            dimension_label: "Month".to_string(),
            price_mode: PriceMode::Off,
            compact: true,
        };

        let result = format_tsv(&data, &totals, &options);
        let lines: Vec<&str> = result.lines().collect();
        assert_eq!(lines[0], "Month\tIn Total\tOut\tTotal");
        assert!(result.ends_with('\n'));
    }
}
