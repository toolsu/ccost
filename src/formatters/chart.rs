use crate::types::{GroupedData, PriceMode, PricedTokenRecord};
use crate::utils::parse_fixed_offset;
use chrono::{DateTime, Local, Utc};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChartModeEnum {
    Cost,
    Token,
}

pub struct ChartOptions {
    pub mode: ChartModeEnum,
    pub dimension_label: String,
    pub price_mode: PriceMode,
    pub tz: Option<String>,
    pub width: Option<usize>,
    pub height: Option<usize>,
}

/// Braille dot map: each character is 2 columns x 4 rows.
/// Index by [row][col] to get the bit to set.
const BRAILLE_DOT_MAP: [[u8; 2]; 4] = [[0x01, 0x08], [0x02, 0x10], [0x04, 0x20], [0x40, 0x80]];

/// Base Unicode codepoint for braille patterns.
const BRAILLE_BASE: u32 = 0x2800;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Granularity {
    Hour,
    Day,
    Month,
}

/// Determine auto granularity based on the labels in grouped data.
/// Examines the label format to determine if they are hours, days, or months.
fn auto_granularity_from_labels(labels: &[&str]) -> Granularity {
    if labels.is_empty() {
        return Granularity::Day;
    }
    // Check label format
    // Hour: "YYYY-MM-DD HH:00" (space) or "YYYY-MM-DDTHH:00" (T)
    // Day: "YYYY-MM-DD" (len 10)
    // Month: "YYYY-MM" (len 7)
    let first = labels[0];
    if first.contains('T') || (first.len() > 10 && first.contains(':')) {
        Granularity::Hour
    } else if first.len() == 10 {
        Granularity::Day
    } else if first.len() == 7 {
        Granularity::Month
    } else {
        Granularity::Day
    }
}

/// Determine auto granularity based on time span of PricedTokenRecords.
fn auto_granularity(records: &[PricedTokenRecord]) -> Granularity {
    if records.len() < 2 {
        return Granularity::Day;
    }
    let first = records.iter().map(|r| r.timestamp).min().unwrap();
    let last = records.iter().map(|r| r.timestamp).max().unwrap();
    let span = last - first;
    let days = span.num_days();

    if days <= 2 {
        Granularity::Hour
    } else if days <= 90 {
        Granularity::Day
    } else {
        Granularity::Month
    }
}

/// Format a timestamp into a bucket key based on granularity and timezone.
fn bucket_key(ts: &DateTime<Utc>, granularity: Granularity, tz: Option<&str>) -> String {
    let formatted = format_in_tz(ts, tz);
    // formatted is "YYYY-MM-DDTHH:MM:SS"
    match granularity {
        Granularity::Hour => {
            // "YYYY-MM-DDTHH"
            if formatted.len() >= 13 {
                formatted[..13].to_string()
            } else {
                formatted
            }
        }
        Granularity::Day => {
            // "YYYY-MM-DD"
            if formatted.len() >= 10 {
                formatted[..10].to_string()
            } else {
                formatted
            }
        }
        Granularity::Month => {
            // "YYYY-MM"
            if formatted.len() >= 7 {
                formatted[..7].to_string()
            } else {
                formatted
            }
        }
    }
}

/// Format a DateTime in the specified timezone, returning "YYYY-MM-DDTHH:MM:SS".
fn format_in_tz(date: &DateTime<Utc>, tz: Option<&str>) -> String {
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

/// Format x-axis label for a bucket key based on granularity.
fn x_label_for_granularity(key: &str, granularity: Granularity) -> String {
    match granularity {
        Granularity::Hour => {
            // key is "YYYY-MM-DDTHH" -> "MM-DDTHH"
            if key.len() >= 13 {
                key[5..13].to_string()
            } else {
                key.to_string()
            }
        }
        Granularity::Day => {
            // key is "YYYY-MM-DD" -> "MM-DD"
            if key.len() >= 10 {
                key[5..10].to_string()
            } else {
                key.to_string()
            }
        }
        Granularity::Month => {
            // key is "YYYY-MM" -> "YYYY-MM"
            key.to_string()
        }
    }
}

/// Format a y-axis label for cost mode.
pub fn y_label_cost(val: f64) -> String {
    if val >= 1000.0 {
        format!("${}k", (val / 1000.0).round() as i64)
    } else if val >= 1.0 {
        format!("${}", val.round() as i64)
    } else {
        format!("${:.2}", val)
    }
}

/// Format a y-axis label for token mode.
fn y_label_token(val: f64) -> String {
    if val >= 1_000_000_000.0 {
        format!("{:.1}G", val / 1_000_000_000.0)
    } else if val >= 1_000_000.0 {
        format!("{:.1}M", val / 1_000_000.0)
    } else if val >= 1_000.0 {
        format!("{}K", (val / 1_000.0).round() as i64)
    } else {
        format!("{}", val.round() as i64)
    }
}

/// Format a y-axis percentage label (0-100%).
pub fn y_label_percent(val: f64) -> String {
    format!("{}%", val.round() as i64)
}

/// Core rendering logic for a braille chart given labels and values.
fn render_chart_core(
    keys: &[String],
    values: &[f64],
    options: &ChartOptions,
    granularity: Granularity,
) -> String {
    if keys.is_empty() || values.is_empty() {
        return "No data to chart.".to_string();
    }

    let max_val = values.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
    let min_val = 0.0_f64; // Always start from 0

    let width = options.width.unwrap_or(80);
    let braille_rows = options.height.unwrap_or(15);

    // Y-axis labels
    let y_label_fn = match options.mode {
        ChartModeEnum::Cost => y_label_cost,
        ChartModeEnum::Token => y_label_token,
    };

    // Generate y-axis label positions: top, bottom, and evenly spaced
    let num_y_labels = braille_rows.clamp(2, 6);
    let mut y_positions: Vec<usize> = Vec::new();
    for i in 0..num_y_labels {
        let pos = if num_y_labels == 1 {
            0
        } else {
            i * (braille_rows - 1) / (num_y_labels - 1)
        };
        y_positions.push(pos);
    }
    y_positions.sort();
    y_positions.dedup();

    // Calculate y-axis label values
    let y_labels: Vec<(usize, String)> = y_positions
        .iter()
        .map(|&pos| {
            let val = if braille_rows <= 1 {
                max_val
            } else {
                max_val - (max_val - min_val) * pos as f64 / (braille_rows - 1) as f64
            };
            (pos, y_label_fn(val))
        })
        .collect();

    let y_label_width = y_labels.iter().map(|(_, l)| l.len()).max().unwrap_or(0);

    // Chart dimensions
    let chart_cols = if width > y_label_width + 3 {
        width - y_label_width - 3
    } else {
        10
    };
    let grid_width = chart_cols * 2;
    let grid_height = braille_rows * 4;

    // Map data points to grid coordinates
    let data_points: Vec<(usize, usize)> = values
        .iter()
        .enumerate()
        .map(|(i, &val)| {
            let x = if values.len() <= 1 {
                0
            } else {
                i * grid_width.saturating_sub(1) / (values.len() - 1)
            };
            let y = if max_val <= min_val {
                grid_height.saturating_sub(1)
            } else {
                let normalized = (val - min_val) / (max_val - min_val);
                let y_pos =
                    ((1.0 - normalized) * (grid_height.saturating_sub(1)) as f64).round() as usize;
                y_pos.min(grid_height.saturating_sub(1))
            };
            (x, y)
        })
        .collect();

    // Initialize braille grid
    let mut grid = vec![vec![0u8; grid_width]; grid_height];

    // Draw lines between consecutive points using Bresenham's algorithm
    for i in 0..data_points.len() {
        let (x0, y0) = data_points[i];
        // Plot the point itself
        if x0 < grid_width && y0 < grid_height {
            grid[y0][x0] = 1;
        }

        if i + 1 < data_points.len() {
            let (x1, y1) = data_points[i + 1];
            bresenham_line(&mut grid, x0, y0, x1, y1, grid_width, grid_height);
        }
    }

    // Convert grid to braille characters
    let mut braille_chars: Vec<Vec<char>> = vec![vec![' '; chart_cols]; braille_rows];

    #[allow(clippy::needless_range_loop)]
    for br in 0..braille_rows {
        for bc in 0..chart_cols {
            let mut pattern: u8 = 0;
            for dr in 0..4 {
                for dc in 0..2 {
                    let gy = br * 4 + dr;
                    let gx = bc * 2 + dc;
                    if gy < grid_height && gx < grid_width && grid[gy][gx] != 0 {
                        pattern |= BRAILLE_DOT_MAP[dr][dc];
                    }
                }
            }
            if pattern != 0 {
                braille_chars[br][bc] =
                    char::from_u32(BRAILLE_BASE + pattern as u32).unwrap_or(' ');
            }
        }
    }

    // Build output
    let mut output = String::new();

    // Title
    let title = match options.mode {
        ChartModeEnum::Cost => "Cost ($)",
        ChartModeEnum::Token => "Tokens",
    };
    output.push_str(title);
    output.push('\n');
    output.push('\n');

    // Build y-axis label map for quick lookup
    let y_label_map: BTreeMap<usize, &str> = y_labels
        .iter()
        .map(|(pos, label)| (*pos, label.as_str()))
        .collect();

    // Chart rows
    #[allow(clippy::needless_range_loop)]
    for br in 0..braille_rows {
        // Y-axis label
        let label = if let Some(label) = y_label_map.get(&br) {
            format!("{:>width$}", label, width = y_label_width)
        } else {
            " ".repeat(y_label_width)
        };
        output.push_str(&label);
        output.push_str(" \u{2524}"); // ┤
        let row_str: String = braille_chars[br].iter().collect();
        output.push_str(&row_str);
        output.push('\n');
    }

    // X-axis border
    output.push_str(&" ".repeat(y_label_width + 1));
    output.push('\u{2514}'); // └
    output.push_str(&"\u{2500}".repeat(chart_cols)); // ─
    output.push('\n');

    // X-axis labels
    let x_labels: Vec<String> = keys
        .iter()
        .map(|k| x_label_for_granularity(k, granularity))
        .collect();

    // Auto-distribute labels across chart width
    let mut label_line = vec![' '; chart_cols];
    if !x_labels.is_empty() {
        let num_labels = x_labels
            .len()
            .min(chart_cols / 8)
            .max(2)
            .min(x_labels.len());
        let indices: Vec<usize> = if num_labels <= 1 {
            vec![0]
        } else {
            (0..num_labels)
                .map(|i| i * (x_labels.len() - 1) / (num_labels - 1))
                .collect()
        };

        for &idx in &indices {
            let label = &x_labels[idx];
            let pos = if x_labels.len() <= 1 {
                0
            } else {
                idx * chart_cols.saturating_sub(1) / (x_labels.len() - 1)
            };
            // Place label at position, truncating if necessary
            let start = pos.min(chart_cols);
            for (j, ch) in label.chars().enumerate() {
                let col = start + j;
                if col < chart_cols {
                    label_line[col] = ch;
                }
            }
        }
    }

    output.push_str(&" ".repeat(y_label_width + 2));
    let label_str: String = label_line.iter().collect();
    output.push_str(label_str.trim_end());
    output.push('\n');

    output
}

/// Render a braille line chart from grouped data (used by main.rs).
/// Each GroupedData entry represents one data point on the chart, keyed by its label.
pub fn render_chart(data: &[GroupedData], _totals: &GroupedData, options: &ChartOptions) -> String {
    if data.is_empty() {
        return "No data to chart.".to_string();
    }

    let labels: Vec<&str> = data.iter().map(|d| d.label.as_str()).collect();
    let granularity = auto_granularity_from_labels(&labels);

    let keys: Vec<String> = data.iter().map(|d| d.label.clone()).collect();
    let values: Vec<f64> = data
        .iter()
        .map(|d| match options.mode {
            ChartModeEnum::Cost => d.total_cost,
            ChartModeEnum::Token => {
                (d.input_tokens + d.output_tokens + d.cache_creation_tokens + d.cache_read_tokens)
                    as f64
            }
        })
        .collect();

    render_chart_core(&keys, &values, options, granularity)
}

/// Render a braille line chart from priced token records (alternative entry point).
/// Aggregates records into time buckets before rendering.
pub fn render_chart_from_records(records: &[PricedTokenRecord], options: &ChartOptions) -> String {
    if records.is_empty() {
        return "No data to chart.".to_string();
    }

    let granularity = auto_granularity(records);
    let tz_ref = options.tz.as_deref();

    // Aggregate data into time buckets
    let mut buckets: BTreeMap<String, f64> = BTreeMap::new();

    for record in records {
        let key = bucket_key(&record.timestamp, granularity, tz_ref);
        let value = match options.mode {
            ChartModeEnum::Cost => record.total_cost,
            ChartModeEnum::Token => {
                (record.input_tokens
                    + record.output_tokens
                    + record.cache_creation_tokens
                    + record.cache_read_tokens) as f64
            }
        };
        *buckets.entry(key).or_insert(0.0) += value;
    }

    if buckets.is_empty() {
        return "No data to chart.".to_string();
    }

    let keys: Vec<String> = buckets.keys().cloned().collect();
    let values: Vec<f64> = buckets.values().cloned().collect();

    render_chart_core(&keys, &values, options, granularity)
}

/// Render a braille chart from raw key-value pairs.
/// Used by sl module for rate-limit and cost charts.
pub fn render_chart_raw(
    keys: &[String],
    values: &[f64],
    title: &str,
    y_label_fn: fn(f64) -> String,
    width: Option<usize>,
    height: Option<usize>,
) -> String {
    if keys.is_empty() || values.is_empty() {
        return "No data to chart.".to_string();
    }

    let max_val = values.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
    let min_val = 0.0_f64; // Always start from 0

    let width = width.unwrap_or(80);
    let braille_rows = height.unwrap_or(15);

    // Generate y-axis label positions: top, bottom, and evenly spaced
    let num_y_labels = braille_rows.clamp(2, 6);
    let mut y_positions: Vec<usize> = Vec::new();
    for i in 0..num_y_labels {
        let pos = if num_y_labels == 1 {
            0
        } else {
            i * (braille_rows - 1) / (num_y_labels - 1)
        };
        y_positions.push(pos);
    }
    y_positions.sort();
    y_positions.dedup();

    // Calculate y-axis label values
    let y_labels: Vec<(usize, String)> = y_positions
        .iter()
        .map(|&pos| {
            let val = if braille_rows <= 1 {
                max_val
            } else {
                max_val - (max_val - min_val) * pos as f64 / (braille_rows - 1) as f64
            };
            (pos, y_label_fn(val))
        })
        .collect();

    let y_label_width = y_labels.iter().map(|(_, l)| l.len()).max().unwrap_or(0);

    // Chart dimensions
    let chart_cols = if width > y_label_width + 3 {
        width - y_label_width - 3
    } else {
        10
    };
    let grid_width = chart_cols * 2;
    let grid_height = braille_rows * 4;

    // Map data points to grid coordinates
    let data_points: Vec<(usize, usize)> = values
        .iter()
        .enumerate()
        .map(|(i, &val)| {
            let x = if values.len() <= 1 {
                0
            } else {
                i * grid_width.saturating_sub(1) / (values.len() - 1)
            };
            let y = if max_val <= min_val {
                grid_height.saturating_sub(1)
            } else {
                let normalized = (val - min_val) / (max_val - min_val);
                let y_pos =
                    ((1.0 - normalized) * (grid_height.saturating_sub(1)) as f64).round() as usize;
                y_pos.min(grid_height.saturating_sub(1))
            };
            (x, y)
        })
        .collect();

    // Initialize braille grid
    let mut grid = vec![vec![0u8; grid_width]; grid_height];

    // Draw lines between consecutive points using Bresenham's algorithm
    for i in 0..data_points.len() {
        let (x0, y0) = data_points[i];
        // Plot the point itself
        if x0 < grid_width && y0 < grid_height {
            grid[y0][x0] = 1;
        }

        if i + 1 < data_points.len() {
            let (x1, y1) = data_points[i + 1];
            bresenham_line(&mut grid, x0, y0, x1, y1, grid_width, grid_height);
        }
    }

    // Convert grid to braille characters
    let mut braille_chars: Vec<Vec<char>> = vec![vec![' '; chart_cols]; braille_rows];

    #[allow(clippy::needless_range_loop)]
    for br in 0..braille_rows {
        for bc in 0..chart_cols {
            let mut pattern: u8 = 0;
            for dr in 0..4 {
                for dc in 0..2 {
                    let gy = br * 4 + dr;
                    let gx = bc * 2 + dc;
                    if gy < grid_height && gx < grid_width && grid[gy][gx] != 0 {
                        pattern |= BRAILLE_DOT_MAP[dr][dc];
                    }
                }
            }
            if pattern != 0 {
                braille_chars[br][bc] =
                    char::from_u32(BRAILLE_BASE + pattern as u32).unwrap_or(' ');
            }
        }
    }

    // Build output
    let mut output = String::new();

    // Title
    output.push_str(title);
    output.push('\n');
    output.push('\n');

    // Build y-axis label map for quick lookup
    let y_label_map: BTreeMap<usize, &str> = y_labels
        .iter()
        .map(|(pos, label)| (*pos, label.as_str()))
        .collect();

    // Chart rows
    #[allow(clippy::needless_range_loop)]
    for br in 0..braille_rows {
        // Y-axis label
        let label = if let Some(label) = y_label_map.get(&br) {
            format!("{:>width$}", label, width = y_label_width)
        } else {
            " ".repeat(y_label_width)
        };
        output.push_str(&label);
        output.push_str(" \u{2524}"); // ┤
        let row_str: String = braille_chars[br].iter().collect();
        output.push_str(&row_str);
        output.push('\n');
    }

    // X-axis border
    output.push_str(&" ".repeat(y_label_width + 1));
    output.push('\u{2514}'); // └
    output.push_str(&"\u{2500}".repeat(chart_cols)); // ─
    output.push('\n');

    // X-axis labels — use keys directly (no granularity-based formatting)
    let x_labels: Vec<&String> = keys.iter().collect();

    // Auto-distribute labels across chart width
    let mut label_line = vec![' '; chart_cols];
    if !x_labels.is_empty() {
        let num_labels = x_labels
            .len()
            .min(chart_cols / 8)
            .max(2)
            .min(x_labels.len());
        let indices: Vec<usize> = if num_labels <= 1 {
            vec![0]
        } else {
            (0..num_labels)
                .map(|i| i * (x_labels.len() - 1) / (num_labels - 1))
                .collect()
        };

        for &idx in &indices {
            let label = x_labels[idx];
            let pos = if x_labels.len() <= 1 {
                0
            } else {
                idx * chart_cols.saturating_sub(1) / (x_labels.len() - 1)
            };
            // Place label at position, truncating if necessary
            let start = pos.min(chart_cols);
            for (j, ch) in label.chars().enumerate() {
                let col = start + j;
                if col < chart_cols {
                    label_line[col] = ch;
                }
            }
        }
    }

    output.push_str(&" ".repeat(y_label_width + 2));
    let label_str: String = label_line.iter().collect();
    output.push_str(label_str.trim_end());
    output.push('\n');

    output
}

/// Draw a line between two points on the grid using Bresenham's algorithm.
fn bresenham_line(
    grid: &mut [Vec<u8>],
    x0: usize,
    y0: usize,
    x1: usize,
    y1: usize,
    max_x: usize,
    max_y: usize,
) {
    let mut x0 = x0 as i64;
    let mut y0 = y0 as i64;
    let x1 = x1 as i64;
    let y1 = y1 as i64;

    let dx = (x1 - x0).abs();
    let dy = -(y1 - y0).abs();
    let sx: i64 = if x0 < x1 { 1 } else { -1 };
    let sy: i64 = if y0 < y1 { 1 } else { -1 };
    let mut err = dx + dy;

    loop {
        if x0 >= 0 && y0 >= 0 && (x0 as usize) < max_x && (y0 as usize) < max_y {
            grid[y0 as usize][x0 as usize] = 1;
        }
        if x0 == x1 && y0 == y1 {
            break;
        }
        let e2 = 2 * err;
        if e2 >= dy {
            if x0 == x1 {
                break;
            }
            err += dy;
            x0 += sx;
        }
        if e2 <= dx {
            if y0 == y1 {
                break;
            }
            err += dx;
            y0 += sy;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn make_record(timestamp: DateTime<Utc>, total_cost: f64, tokens: u64) -> PricedTokenRecord {
        PricedTokenRecord {
            timestamp,
            model: "test".to_string(),
            session_id: "s1".to_string(),
            project: "p1".to_string(),
            input_tokens: tokens,
            output_tokens: tokens,
            cache_creation_tokens: 0,
            cache_read_tokens: 0,
            input_cost: total_cost / 2.0,
            cache_creation_cost: 0.0,
            cache_read_cost: 0.0,
            output_cost: total_cost / 2.0,
            total_cost,
        }
    }

    fn make_grouped(label: &str, total_cost: f64, tokens: u64) -> GroupedData {
        GroupedData {
            label: label.to_string(),
            input_tokens: tokens,
            output_tokens: tokens,
            cache_creation_tokens: 0,
            cache_read_tokens: 0,
            input_cost: total_cost / 2.0,
            cache_creation_cost: 0.0,
            cache_read_cost: 0.0,
            output_cost: total_cost / 2.0,
            total_cost,
            children: None,
        }
    }

    #[test]
    fn test_render_chart_empty() {
        let totals = make_grouped("TOTAL", 0.0, 0);
        let options = ChartOptions {
            mode: ChartModeEnum::Cost,
            dimension_label: "Date".to_string(),
            price_mode: PriceMode::Integer,
            tz: Some("UTC".to_string()),
            width: None,
            height: None,
        };
        assert_eq!(render_chart(&[], &totals, &options), "No data to chart.");
    }

    #[test]
    fn test_render_chart_single_point() {
        let data = vec![make_grouped("2025-01-15", 5.0, 1000)];
        let totals = data[0].clone();
        let options = ChartOptions {
            mode: ChartModeEnum::Cost,
            dimension_label: "Date".to_string(),
            price_mode: PriceMode::Integer,
            tz: Some("UTC".to_string()),
            width: Some(40),
            height: Some(5),
        };
        let result = render_chart(&data, &totals, &options);
        assert!(result.contains("Cost ($)"));
        assert!(result.contains("\u{2524}")); // ┤
        assert!(result.contains("\u{2514}")); // └
    }

    #[test]
    fn test_render_chart_multiple_points() {
        let data = vec![
            make_grouped("2025-01-01", 1.0, 100),
            make_grouped("2025-01-02", 3.0, 300),
            make_grouped("2025-01-03", 2.0, 200),
        ];
        let totals = make_grouped("TOTAL", 6.0, 600);
        let options = ChartOptions {
            mode: ChartModeEnum::Token,
            dimension_label: "Date".to_string(),
            price_mode: PriceMode::Off,
            tz: Some("UTC".to_string()),
            width: Some(40),
            height: Some(5),
        };
        let result = render_chart(&data, &totals, &options);
        assert!(result.contains("Tokens"));
    }

    #[test]
    fn test_render_chart_from_records_empty() {
        let options = ChartOptions {
            mode: ChartModeEnum::Cost,
            dimension_label: "Date".to_string(),
            price_mode: PriceMode::Integer,
            tz: Some("UTC".to_string()),
            width: None,
            height: None,
        };
        assert_eq!(
            render_chart_from_records(&[], &options),
            "No data to chart."
        );
    }

    #[test]
    fn test_render_chart_from_records_multiple() {
        let records = vec![
            make_record(Utc.with_ymd_and_hms(2025, 1, 1, 0, 0, 0).unwrap(), 1.0, 100),
            make_record(Utc.with_ymd_and_hms(2025, 1, 2, 0, 0, 0).unwrap(), 3.0, 300),
            make_record(Utc.with_ymd_and_hms(2025, 1, 3, 0, 0, 0).unwrap(), 2.0, 200),
        ];
        let options = ChartOptions {
            mode: ChartModeEnum::Cost,
            dimension_label: "Date".to_string(),
            price_mode: PriceMode::Integer,
            tz: Some("UTC".to_string()),
            width: Some(40),
            height: Some(5),
        };
        let result = render_chart_from_records(&records, &options);
        assert!(result.contains("Cost ($)"));
    }

    #[test]
    fn test_auto_granularity() {
        // Within 2 days -> Hour
        let records = vec![
            make_record(Utc.with_ymd_and_hms(2025, 1, 1, 0, 0, 0).unwrap(), 1.0, 100),
            make_record(Utc.with_ymd_and_hms(2025, 1, 2, 0, 0, 0).unwrap(), 1.0, 100),
        ];
        assert_eq!(auto_granularity(&records), Granularity::Hour);

        // Within 90 days -> Day
        let records = vec![
            make_record(Utc.with_ymd_and_hms(2025, 1, 1, 0, 0, 0).unwrap(), 1.0, 100),
            make_record(Utc.with_ymd_and_hms(2025, 3, 1, 0, 0, 0).unwrap(), 1.0, 100),
        ];
        assert_eq!(auto_granularity(&records), Granularity::Day);

        // Over 90 days -> Month
        let records = vec![
            make_record(Utc.with_ymd_and_hms(2025, 1, 1, 0, 0, 0).unwrap(), 1.0, 100),
            make_record(Utc.with_ymd_and_hms(2025, 7, 1, 0, 0, 0).unwrap(), 1.0, 100),
        ];
        assert_eq!(auto_granularity(&records), Granularity::Month);
    }

    #[test]
    fn test_auto_granularity_from_labels() {
        assert_eq!(
            auto_granularity_from_labels(&["2025-01-15T10"]),
            Granularity::Hour
        );
        assert_eq!(
            auto_granularity_from_labels(&["2025-01-15"]),
            Granularity::Day
        );
        assert_eq!(
            auto_granularity_from_labels(&["2025-01"]),
            Granularity::Month
        );
    }

    #[test]
    fn test_y_label_cost() {
        assert_eq!(y_label_cost(1500.0), "$2k");
        assert_eq!(y_label_cost(5.0), "$5");
        assert_eq!(y_label_cost(0.5), "$0.50");
    }

    #[test]
    fn test_y_label_token() {
        assert_eq!(y_label_token(2_000_000_000.0), "2.0G");
        assert_eq!(y_label_token(1_500_000.0), "1.5M");
        assert_eq!(y_label_token(5000.0), "5K");
        assert_eq!(y_label_token(500.0), "500");
    }

    #[test]
    fn test_bucket_key() {
        let ts = Utc.with_ymd_and_hms(2025, 3, 15, 10, 30, 0).unwrap();
        assert_eq!(
            bucket_key(&ts, Granularity::Hour, Some("UTC")),
            "2025-03-15T10"
        );
        assert_eq!(bucket_key(&ts, Granularity::Day, Some("UTC")), "2025-03-15");
        assert_eq!(bucket_key(&ts, Granularity::Month, Some("UTC")), "2025-03");
    }

    #[test]
    fn test_x_label_for_granularity() {
        assert_eq!(
            x_label_for_granularity("2025-03-15T10", Granularity::Hour),
            "03-15T10"
        );
        assert_eq!(
            x_label_for_granularity("2025-03-15", Granularity::Day),
            "03-15"
        );
        assert_eq!(
            x_label_for_granularity("2025-03", Granularity::Month),
            "2025-03"
        );
    }
}
