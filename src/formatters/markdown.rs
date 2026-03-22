use super::table::{format_cost, format_tokens};
use crate::types::{GroupedData, PriceMode};

pub struct MarkdownOptions {
    pub dimension_label: String,
    pub price_mode: PriceMode,
    pub compact: bool,
}

/// Escape pipe characters in cell content for Markdown tables.
fn escape_pipe(s: &str) -> String {
    s.replace('|', "\\|")
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

fn build_row(
    entry: &GroupedData,
    price_mode: PriceMode,
    compact: bool,
    label_prefix: &str,
) -> RowData {
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
            format_cell(
                entry.cache_creation_tokens,
                entry.cache_creation_cost,
                price_mode,
            ),
            format_cell(entry.cache_read_tokens, entry.cache_read_cost, price_mode),
            format_cell(in_total, in_total_cost, price_mode),
            format_cell(entry.output_tokens, entry.output_cost, price_mode),
            format_cell(total, total_cost, price_mode),
        ]
    };

    RowData { label, cells }
}

pub fn format_markdown(
    data: &[GroupedData],
    totals: &GroupedData,
    options: &MarkdownOptions,
) -> String {
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

    let mut output = String::new();

    // Header row
    output.push_str("| ");
    output.push_str(
        &headers
            .iter()
            .map(|h| escape_pipe(h))
            .collect::<Vec<_>>()
            .join(" | "),
    );
    output.push_str(" |\n");

    // Separator row
    output.push_str("| ");
    output.push_str(
        &headers
            .iter()
            .map(|h| {
                let width = h.len().max(3);
                "-".repeat(width)
            })
            .collect::<Vec<_>>()
            .join(" | "),
    );
    output.push_str(" |\n");

    // Data rows
    for entry in data {
        let row = build_row(entry, options.price_mode, options.compact, "");
        output.push_str("| ");
        output.push_str(&escape_pipe(&row.label));
        for cell in &row.cells {
            output.push_str(" | ");
            output.push_str(&escape_pipe(cell));
        }
        output.push_str(" |\n");

        if let Some(ref children) = entry.children {
            for child in children {
                let child_row = build_row(
                    child,
                    options.price_mode,
                    options.compact,
                    "\u{2514}\u{2500} ",
                );
                output.push_str("| ");
                output.push_str(&escape_pipe(&child_row.label));
                for cell in &child_row.cells {
                    output.push_str(" | ");
                    output.push_str(&escape_pipe(cell));
                }
                output.push_str(" |\n");
            }
        }
    }

    // Totals row
    let totals_row = build_row(totals, options.price_mode, options.compact, "");
    output.push_str("| TOTAL");
    for cell in &totals_row.cells {
        output.push_str(" | ");
        output.push_str(&escape_pipe(cell));
    }
    output.push_str(" |\n");

    // Totals children
    if let Some(ref children) = totals.children {
        for child in children {
            let child_row = build_row(
                child,
                options.price_mode,
                options.compact,
                "\u{2514}\u{2500} ",
            );
            output.push_str("| ");
            output.push_str(&escape_pipe(&child_row.label));
            for cell in &child_row.cells {
                output.push_str(" | ");
                output.push_str(&escape_pipe(cell));
            }
            output.push_str(" |\n");
        }
    }

    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_markdown_basic() {
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

        let options = MarkdownOptions {
            dimension_label: "Month".to_string(),
            price_mode: PriceMode::Off,
            compact: false,
        };

        let result = format_markdown(&data, &totals, &options);
        assert!(result.contains("| Month |"));
        assert!(result.contains("| 2025-01 |"));
        assert!(result.contains("| TOTAL |"));
        assert!(result.ends_with('\n'));
    }

    #[test]
    fn test_format_markdown_compact() {
        let data = vec![GroupedData {
            label: "test".to_string(),
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

        let options = MarkdownOptions {
            dimension_label: "Model".to_string(),
            price_mode: PriceMode::Off,
            compact: true,
        };

        let result = format_markdown(&data, &totals, &options);
        assert!(!result.contains("Cache"));
        assert!(result.contains("In Total"));
    }

    #[test]
    fn test_escape_pipe() {
        assert_eq!(escape_pipe("hello|world"), "hello\\|world");
        assert_eq!(escape_pipe("no pipes"), "no pipes");
    }
}
