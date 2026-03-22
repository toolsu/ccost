use crate::types::{GroupedData, PriceMode};

pub struct TableOptions {
    pub dimension_label: String,
    pub price_mode: PriceMode,
    pub compact: bool,
    pub color: Option<bool>, // None means true (default color on)
}

/// Format a token count for display.
/// - n >= 1_000_000: "X.XXM"
/// - n >= 1_000: "X.XK"
/// - otherwise: plain number
pub fn format_tokens(n: u64) -> String {
    if n >= 1_000_000 {
        format!("{:.2}M", n as f64 / 1_000_000.0)
    } else if n >= 1_000 {
        format!("{:.1}K", n as f64 / 1_000.0)
    } else {
        n.to_string()
    }
}

/// Format a cost value for display.
/// - Integer mode: "$X" (rounded)
/// - Decimal mode: "$X.XX"
/// - Off mode: empty string
pub fn format_cost(n: f64, mode: PriceMode) -> String {
    match mode {
        PriceMode::Integer => format!("${}", n.round() as i64),
        PriceMode::Decimal => format!("${:.2}", n),
        PriceMode::Off => String::new(),
    }
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

fn collect_rows(data: &[GroupedData], price_mode: PriceMode, compact: bool) -> Vec<RowData> {
    let mut rows = Vec::new();
    for entry in data {
        rows.push(build_row(entry, price_mode, compact, ""));
        if let Some(ref children) = entry.children {
            for child in children {
                rows.push(build_row(child, price_mode, compact, "\u{2514}\u{2500} "));
            }
        }
    }
    rows
}

pub fn format_table(data: &[GroupedData], totals: &GroupedData, options: &TableOptions) -> String {
    let headers: Vec<String> = if options.compact {
        vec![
            options.dimension_label.clone(),
            "In Total".to_string(),
            "Out".to_string(),
            "Total".to_string(),
        ]
    } else {
        vec![
            options.dimension_label.clone(),
            "In".to_string(),
            "Cache Cr".to_string(),
            "Cache Rd".to_string(),
            "In Total".to_string(),
            "Out".to_string(),
            "Total".to_string(),
        ]
    };

    let data_rows = collect_rows(data, options.price_mode, options.compact);

    // Build totals rows
    let mut totals_rows = Vec::new();
    totals_rows.push(build_row(totals, options.price_mode, options.compact, ""));
    // Override label to "TOTAL"
    totals_rows[0].label = "TOTAL".to_string();
    if let Some(ref children) = totals.children {
        for child in children {
            totals_rows.push(build_row(child, options.price_mode, options.compact, "\u{2514}\u{2500} "));
        }
    }

    let num_cols = headers.len();

    // Calculate column widths
    let mut col_widths: Vec<usize> = headers.iter().map(|h| display_width(h)).collect();

    for row in data_rows.iter().chain(totals_rows.iter()) {
        let label_width = display_width(&row.label);
        if label_width > col_widths[0] {
            col_widths[0] = label_width;
        }
        for (i, cell) in row.cells.iter().enumerate() {
            let w = display_width(cell);
            if w > col_widths[i + 1] {
                col_widths[i + 1] = w;
            }
        }
    }

    let color_enabled = options.color != Some(false);

    let mut output = String::new();

    // Top border: ┌─┬─┐
    output.push('\u{250C}');
    for (i, &w) in col_widths.iter().enumerate() {
        output.push_str(&"\u{2500}".repeat(w + 2));
        if i < num_cols - 1 {
            output.push('\u{252C}');
        }
    }
    output.push('\u{2510}');
    output.push('\n');

    // Header row
    output.push('\u{2502}');
    for (i, header) in headers.iter().enumerate() {
        if i == 0 {
            // Left-aligned
            output.push(' ');
            output.push_str(header);
            output.push_str(&" ".repeat(col_widths[i] - display_width(header) + 1));
        } else {
            // Right-aligned
            output.push_str(&" ".repeat(col_widths[i] - display_width(header) + 1));
            output.push_str(header);
            output.push(' ');
        }
        output.push('\u{2502}');
    }
    output.push('\n');

    // Mid separator after header
    output.push_str(&mid_separator(&col_widths));
    output.push('\n');

    // Data rows
    for (row_idx, row) in data_rows.iter().enumerate() {
        output.push('\u{2502}');
        // Label column (left-aligned)
        output.push(' ');
        output.push_str(&row.label);
        output.push_str(&" ".repeat(col_widths[0] - display_width(&row.label) + 1));
        output.push('\u{2502}');
        // Data columns (right-aligned)
        for (i, cell) in row.cells.iter().enumerate() {
            output.push_str(&" ".repeat(col_widths[i + 1] - display_width(cell) + 1));
            output.push_str(cell);
            output.push(' ');
            output.push('\u{2502}');
        }
        output.push('\n');

        // Mid separator between rows
        if row_idx < data_rows.len() - 1 {
            output.push_str(&mid_separator(&col_widths));
            output.push('\n');
        }
    }

    // Mid separator before totals
    if !data_rows.is_empty() {
        output.push_str(&mid_separator(&col_widths));
        output.push('\n');
    }

    // Totals rows
    for (row_idx, row) in totals_rows.iter().enumerate() {
        output.push('\u{2502}');

        let green_start = if color_enabled { "\x1b[33m" } else { "" };
        let green_end = if color_enabled { "\x1b[0m" } else { "" };

        // Label column (left-aligned)
        output.push(' ');
        output.push_str(green_start);
        output.push_str(&row.label);
        output.push_str(green_end);
        output.push_str(&" ".repeat(col_widths[0] - display_width(&row.label) + 1));
        output.push('\u{2502}');

        // Data columns (right-aligned)
        for (i, cell) in row.cells.iter().enumerate() {
            output.push_str(&" ".repeat(col_widths[i + 1] - display_width(cell) + 1));
            output.push_str(green_start);
            output.push_str(cell);
            output.push_str(green_end);
            output.push(' ');
            output.push('\u{2502}');
        }
        output.push('\n');

        // Mid separator between totals rows
        if row_idx < totals_rows.len() - 1 {
            output.push_str(&mid_separator(&col_widths));
            output.push('\n');
        }
    }

    // Bottom border: └─┴─┘
    output.push('\u{2514}');
    for (i, &w) in col_widths.iter().enumerate() {
        output.push_str(&"\u{2500}".repeat(w + 2));
        if i < num_cols - 1 {
            output.push('\u{2534}');
        }
    }
    output.push('\u{2518}');
    output.push('\n');

    output
}

fn mid_separator(col_widths: &[usize]) -> String {
    let mut s = String::new();
    s.push('\u{251C}');
    for (i, &w) in col_widths.iter().enumerate() {
        s.push_str(&"\u{2500}".repeat(w + 2));
        if i < col_widths.len() - 1 {
            s.push('\u{253C}');
        }
    }
    s.push('\u{2524}');
    s
}

/// Calculate the display width of a string, accounting for Unicode characters.
/// Box-drawing and other wide characters are counted as 1 for simplicity,
/// since terminals render them as single-width.
fn display_width(s: &str) -> usize {
    s.chars().count()
}

/// Format grouped data as a plain-text table (no ANSI colors).
/// This is the "txt" format equivalent, a convenience wrapper around `format_table`
/// with colors forced off.
pub fn format_txt(
    data: &[GroupedData],
    totals: &GroupedData,
    opts: &TableOptions,
) -> String {
    let txt_opts = TableOptions {
        dimension_label: opts.dimension_label.clone(),
        price_mode: opts.price_mode,
        compact: opts.compact,
        color: Some(false),
    };
    format_table(data, totals, &txt_opts)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_tokens() {
        assert_eq!(format_tokens(999), "999");
        assert_eq!(format_tokens(1000), "1.0K");
        assert_eq!(format_tokens(1500), "1.5K");
        assert_eq!(format_tokens(1_000_000), "1.00M");
        assert_eq!(format_tokens(1_500_000), "1.50M");
    }

    #[test]
    fn test_format_cost() {
        assert_eq!(format_cost(1.0, PriceMode::Integer), "$1");
        assert_eq!(format_cost(1.234, PriceMode::Decimal), "$1.23");
        assert_eq!(format_cost(1.0, PriceMode::Off), "");
    }

    #[test]
    fn test_format_table_basic() {
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

        let options = TableOptions {
            dimension_label: "Month".to_string(),
            price_mode: PriceMode::Off,
            compact: false,
            color: Some(false),
        };

        let result = format_table(&data, &totals, &options);
        assert!(result.contains("2025-01"));
        assert!(result.contains("TOTAL"));
    }

    #[test]
    fn test_format_txt_no_ansi() {
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

        // Even if color is Some(true) in the input opts, format_txt forces it off
        let options = TableOptions {
            dimension_label: "Month".to_string(),
            price_mode: PriceMode::Off,
            compact: false,
            color: Some(true),
        };

        let result = format_txt(&data, &totals, &options);
        assert!(result.contains("2025-01"));
        assert!(result.contains("TOTAL"));
        assert!(
            !result.contains("\x1b["),
            "format_txt should not contain ANSI escape codes"
        );
    }
}
