use super::table::{format_cost, format_tokens};
use crate::types::{GroupedData, PriceMode};

pub struct HtmlOptions {
    pub dimension_label: String,
    pub price_mode: PriceMode,
    pub compact: bool,
    pub title: Option<String>,
}

/// HTML-escape user content.
fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// Format a cell value: tokens with optional cost in a span.
fn format_cell_html(tokens: u64, cost: f64, price_mode: PriceMode) -> String {
    let token_str = html_escape(&format_tokens(tokens));
    if price_mode == PriceMode::Off {
        token_str
    } else {
        let cost_str = html_escape(&format_cost(cost, price_mode));
        format!("{} <span class=\"cost\">({})</span>", token_str, cost_str)
    }
}

struct RowData {
    label: String,
    cells: Vec<String>,
}

fn build_row_html(
    entry: &GroupedData,
    price_mode: PriceMode,
    compact: bool,
    label_prefix: &str,
) -> RowData {
    let in_total = entry.input_tokens + entry.cache_creation_tokens + entry.cache_read_tokens;
    let in_total_cost = entry.input_cost + entry.cache_creation_cost + entry.cache_read_cost;
    let total = in_total + entry.output_tokens;
    let total_cost = entry.total_cost;

    let label = format!("{}{}", label_prefix, html_escape(&entry.label));

    let cells = if compact {
        vec![
            format_cell_html(in_total, in_total_cost, price_mode),
            format_cell_html(entry.output_tokens, entry.output_cost, price_mode),
            format_cell_html(total, total_cost, price_mode),
        ]
    } else {
        vec![
            format_cell_html(entry.input_tokens, entry.input_cost, price_mode),
            format_cell_html(
                entry.cache_creation_tokens,
                entry.cache_creation_cost,
                price_mode,
            ),
            format_cell_html(entry.cache_read_tokens, entry.cache_read_cost, price_mode),
            format_cell_html(in_total, in_total_cost, price_mode),
            format_cell_html(entry.output_tokens, entry.output_cost, price_mode),
            format_cell_html(total, total_cost, price_mode),
        ]
    };

    RowData { label, cells }
}

pub fn format_html(data: &[GroupedData], totals: &GroupedData, options: &HtmlOptions) -> String {
    let title = options.title.as_deref().unwrap_or("ccost report");
    let is_default_title = options.title.is_none();

    let headers: Vec<&str> = if options.compact {
        vec![&options.dimension_label, "Input Total", "Output", "Total"]
    } else {
        vec![
            &options.dimension_label,
            "Input",
            "Cache Creation",
            "Cache Read",
            "Input Total",
            "Output",
            "Total",
        ]
    };

    let num_cols = headers.len();

    let mut html = String::new();

    // DOCTYPE and head
    html.push_str("<!DOCTYPE html>\n<html lang=\"en\">\n<head>\n<meta charset=\"UTF-8\">\n<meta name=\"viewport\" content=\"width=device-width, initial-scale=1.0\">\n<title>");
    html.push_str(&html_escape(title));
    html.push_str("</title>\n<style>\n");
    html.push_str(CSS);
    html.push_str("\n</style>\n</head>\n<body>\n");

    // h1
    if is_default_title {
        html.push_str("<h1><a href=\"https://github.com/cc-friend/ccost\">");
        html.push_str(&html_escape(title));
        html.push_str("</a></h1>\n");
    } else {
        html.push_str("<h1>");
        html.push_str(&html_escape(title));
        html.push_str("</h1>\n");
    }

    // Table
    html.push_str("<table>\n<thead>\n<tr>\n");
    for (i, header) in headers.iter().enumerate() {
        html.push_str(&format!(
            "<th class=\"sortable\" data-col=\"{}\">{}<span class=\"sort-arrow\"><svg width=\"12\" height=\"14\" viewBox=\"0 0 12 14\"><path d=\"M6 0L12 6H0z\" class=\"arrow-up\"/><path d=\"M6 14L0 8h12z\" class=\"arrow-down\"/></svg></span></th>\n",
            i,
            html_escape(header)
        ));
    }
    html.push_str("</tr>\n</thead>\n<tbody>\n");

    // Data rows
    for entry in data {
        let row = build_row_html(entry, options.price_mode, options.compact, "");
        html.push_str("<tr class=\"parent\">\n");
        html.push_str(&format!("<td>{}</td>\n", row.label));
        for cell in &row.cells {
            html.push_str(&format!("<td>{}</td>\n", cell));
        }
        html.push_str("</tr>\n");

        if let Some(ref children) = entry.children {
            for child in children {
                let child_row = build_row_html(
                    child,
                    options.price_mode,
                    options.compact,
                    "\u{2514}\u{2500} ",
                );
                html.push_str("<tr class=\"child\">\n");
                html.push_str(&format!("<td>{}</td>\n", child_row.label));
                for cell in &child_row.cells {
                    html.push_str(&format!("<td>{}</td>\n", cell));
                }
                html.push_str("</tr>\n");
            }
        }
    }

    html.push_str("</tbody>\n<tfoot>\n");

    // Totals row
    let totals_row = build_row_html(totals, options.price_mode, options.compact, "");
    html.push_str("<tr class=\"totals totals-main\">\n");
    html.push_str("<td>TOTAL</td>\n");
    for cell in &totals_row.cells {
        html.push_str(&format!("<td>{}</td>\n", cell));
    }
    html.push_str("</tr>\n");

    // Totals children
    if let Some(ref children) = totals.children {
        for child in children {
            let child_row = build_row_html(
                child,
                options.price_mode,
                options.compact,
                "\u{2514}\u{2500} ",
            );
            html.push_str("<tr class=\"totals totals-child\">\n");
            html.push_str(&format!("<td>{}</td>\n", child_row.label));
            for cell in &child_row.cells {
                html.push_str(&format!("<td>{}</td>\n", cell));
            }
            html.push_str("</tr>\n");
        }
    }

    html.push_str("</tfoot>\n</table>\n");

    // JavaScript
    html.push_str("<script>\n");
    html.push_str(&build_js(num_cols));
    html.push_str("\n</script>\n");

    html.push_str("</body>\n</html>\n");

    html
}

const CSS: &str = r#"* {
  margin: 0;
  padding: 0;
  box-sizing: border-box;
}
body {
  background: #1a1816;
  color: #e0e0e0;
  font-family: 'Segoe UI', Tahoma, Geneva, Verdana, sans-serif;
  padding: 2rem;
}
h1 {
  color: #D4795A;
  margin-bottom: 1.5rem;
  font-size: 1.5rem;
}
h1 a {
  color: #D4795A;
  text-decoration: none;
}
h1 a:hover {
  text-decoration: underline;
}
table {
  border-collapse: collapse;
  width: 100%;
  font-size: 0.9rem;
}
th, td {
  padding: 0.6rem 1rem;
  border: 1px solid #333;
  text-align: right;
}
th:first-child, td:first-child {
  text-align: left;
}
thead th {
  background: #2a2520;
  color: #D4795A;
  cursor: pointer;
  user-select: none;
  white-space: nowrap;
}
thead th:hover {
  background: #3a3530;
}
tbody, tfoot {
  font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', monospace;
}
tbody tr.parent {
  background: #222;
}
tbody tr.parent:hover {
  background: #2a2a2a;
}
tbody tr.child {
  background: #1e1e1e;
  font-size: 0.85rem;
}
tbody tr.child td:first-child {
  padding-left: 2rem;
}
tfoot tr.totals {
  background: #2a2520;
  font-weight: bold;
}
tfoot tr.totals-main {
  color: #D4795A;
}
tfoot tr.totals-child {
  font-weight: normal;
  font-size: 0.85rem;
}
tfoot tr.totals-child td:first-child {
  padding-left: 2rem;
}
.cost {
  color: #6ee7b7;
  font-size: 0.85em;
}
.sort-arrow {
  display: inline-block;
  margin-left: 4px;
  vertical-align: middle;
}
.sort-arrow svg {
  display: block;
}
.arrow-up, .arrow-down {
  fill: #555;
  transition: fill 0.2s;
}
th.sort-asc .arrow-up {
  fill: #D4795A;
}
th.sort-desc .arrow-down {
  fill: #D4795A;
}"#;

fn build_js(_num_cols: usize) -> String {
    r#"(function() {
  const table = document.querySelector('table');
  const thead = table.querySelector('thead');
  const tbody = table.querySelector('tbody');
  const ths = thead.querySelectorAll('th');
  let sortState = {};

  function getGroups() {
    const rows = Array.from(tbody.querySelectorAll('tr'));
    const groups = [];
    let current = null;
    for (const row of rows) {
      const firstCell = row.querySelector('td');
      const text = firstCell ? firstCell.textContent : '';
      if (row.classList.contains('child') || text.startsWith('\u{2514}')) {
        if (current) current.children.push(row);
      } else {
        current = { parent: row, children: [] };
        groups.push(current);
      }
    }
    return groups;
  }

  function sfx(n, s) {
    if (!s) return n;
    s = s.toUpperCase();
    if (s === 'K') return n * 1e3;
    if (s === 'M') return n * 1e6;
    if (s === 'G' || s === 'B') return n * 1e9;
    return n;
  }

  function parseValue(text) {
    const t = text.replace(/\(.*?\)/g, '').trim();
    if (t === '\u2014' || t === '' || t === '-') return NaN;
    // Dollar: $1.23 or $1.2K
    let m = t.match(/^\$([\d,.]+)\s*([KMGB])?$/i);
    if (m) return sfx(parseFloat(m[1].replace(/,/g, '')), m[2]);
    // Duration: 1d 2h 30m 15s (any combo)
    m = t.match(/^(?:(\d+)d\s*)?(?:(\d+)h\s*)?(?:(\d+)m\s*)?(?:(\d+)s)?$/);
    if (m && (m[1]||m[2]||m[3]||m[4]))
      return ((+m[1]||0)*86400)+((+m[2]||0)*3600)+((+m[3]||0)*60)+(+m[4]||0);
    // Pct range: 10%–25% or 10% — sort by max
    m = t.match(/([\d.]+)%/g);
    if (m) return parseFloat(m[m.length - 1]);
    // Lines: +123 -45
    m = t.match(/^\+([\d,]+)\s+-([\d,]+)$/);
    if (m) return parseInt(m[1].replace(/,/g,'')) + parseInt(m[2].replace(/,/g,''));
    // Plain number with optional suffix: 1,200 or 1.2K
    m = t.match(/^([\d,.]+)\s*([KMGB])?$/i);
    if (m) return sfx(parseFloat(m[1].replace(/,/g, '')), m[2]);
    return NaN;
  }

  function getCellValue(row, col) {
    const cells = row.querySelectorAll('td');
    if (col >= cells.length) return '';
    return cells[col].textContent || '';
  }

  const originalGroups = getGroups().map((g, i) => ({ ...g, index: i }));

  ths.forEach((th, colIdx) => {
    th.addEventListener('click', () => {
      const prev = sortState[colIdx] || 'none';
      let next;
      if (prev === 'none') next = 'asc';
      else if (prev === 'asc') next = 'desc';
      else next = 'none';

      // Clear all sort classes
      ths.forEach(t => {
        t.classList.remove('sort-asc', 'sort-desc');
      });
      sortState = {};

      if (next !== 'none') {
        sortState[colIdx] = next;
        th.classList.add('sort-' + next);
      }

      let groups = originalGroups.map(g => ({ ...g }));

      if (next !== 'none') {
        groups.sort((a, b) => {
          const aText = getCellValue(a.parent, colIdx);
          const bText = getCellValue(b.parent, colIdx);
          const aNum = parseValue(aText);
          const bNum = parseValue(bText);
          let cmp;
          if (!isNaN(aNum) && !isNaN(bNum)) {
            cmp = aNum - bNum;
          } else {
            cmp = aText.localeCompare(bText);
          }
          return next === 'desc' ? -cmp : cmp;
        });
      }

      // Rebuild tbody
      while (tbody.firstChild) tbody.removeChild(tbody.firstChild);
      for (const g of groups) {
        tbody.appendChild(g.parent);
        for (const child of g.children) {
          tbody.appendChild(child);
        }
      }
    });
  });
})();"#
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_html_escape() {
        assert_eq!(html_escape("<script>"), "&lt;script&gt;");
        assert_eq!(html_escape("a&b"), "a&amp;b");
        assert_eq!(html_escape("say \"hi\""), "say &quot;hi&quot;");
    }

    #[test]
    fn test_format_html_basic() {
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

        let options = HtmlOptions {
            dimension_label: "Month".to_string(),
            price_mode: PriceMode::Off,
            compact: false,
            title: None,
        };

        let result = format_html(&data, &totals, &options);
        assert!(result.contains("<!DOCTYPE html>"));
        assert!(result.contains("ccost report"));
        assert!(result.contains("https://github.com/cc-friend/ccost"));
        assert!(result.contains("<thead>"));
        assert!(result.contains("<tbody>"));
        assert!(result.contains("<tfoot>"));
        assert!(result.contains("class=\"parent\""));
        assert!(result.contains("class=\"totals totals-main\""));
        assert!(result.contains("sortable"));
    }

    #[test]
    fn test_format_html_custom_title() {
        let data = vec![];
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

        let options = HtmlOptions {
            dimension_label: "Month".to_string(),
            price_mode: PriceMode::Off,
            compact: true,
            title: Some("My Custom Report".to_string()),
        };

        let result = format_html(&data, &totals, &options);
        assert!(result.contains("My Custom Report"));
        // Custom title should NOT have the link
        assert!(!result.contains("https://github.com/cc-friend/ccost"));
    }

    #[test]
    fn test_html_css_tbody_font() {
        let html = CSS;
        assert!(
            html.contains("tbody") && html.contains("tfoot"),
            "CSS should have tbody/tfoot rule"
        );
        assert!(
            html.contains("-apple-system") && html.contains("monospace"),
            "tbody/tfoot should use -apple-system, monospace font stack"
        );
    }

    #[test]
    fn test_html_css_cost_color() {
        let html = CSS;
        assert!(
            html.contains(".cost {") && html.contains("color: #6ee7b7;"),
            "cost spans should use color #6ee7b7"
        );
    }

    #[test]
    fn test_html_sort_arrow_spacing() {
        let data = vec![];
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
        let options = HtmlOptions {
            dimension_label: "Day".to_string(),
            price_mode: PriceMode::Off,
            compact: false,
            title: None,
        };
        let result = format_html(&data, &totals, &options);
        // SVG height should be 14 (not 12) to have gap between arrows
        assert!(
            result.contains("height=\"14\""),
            "sort arrow SVG should have height 14 for spacing"
        );
        // Down arrow should start at y=8 (gap from y=6 to y=8)
        assert!(
            result.contains("M6 14L0 8h12z"),
            "down arrow path should be offset to create gap"
        );
    }

    #[test]
    fn test_html_cost_span_color() {
        let data = vec![GroupedData {
            label: "2025-01".to_string(),
            input_tokens: 1000,
            output_tokens: 500,
            cache_creation_tokens: 0,
            cache_read_tokens: 0,
            input_cost: 5.0,
            cache_creation_cost: 0.0,
            cache_read_cost: 0.0,
            output_cost: 5.0,
            total_cost: 10.0,
            children: None,
        }];
        let totals = data[0].clone();
        let options = HtmlOptions {
            dimension_label: "Month".to_string(),
            price_mode: PriceMode::Integer,
            compact: true,
            title: None,
        };
        let result = format_html(&data, &totals, &options);
        assert!(
            result.contains("<span class=\"cost\">"),
            "should contain cost spans when price_mode is not Off"
        );
    }
}
