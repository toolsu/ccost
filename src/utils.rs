use std::io::{IsTerminal, Write};

use chrono::{FixedOffset, NaiveDateTime};

/// Returns the current terminal width.
///
/// Priority:
/// 1. `COLUMNS` env var (explicit user/test override)
/// 2. 200 when stdout is not a terminal (piped)
/// 3. Actual terminal size via `terminal_size`
/// 4. Fallback: 80
pub fn term_width() -> usize {
    // COLUMNS env var takes priority (explicit user/test override)
    if let Ok(cols) = std::env::var("COLUMNS") {
        if let Ok(n) = cols.parse::<usize>() {
            if n > 0 {
                return n;
            }
        }
    }
    // When piped (not a terminal), default to wide
    if !std::io::stdout().is_terminal() {
        return 200;
    }
    // Query actual terminal size
    if let Some((terminal_size::Width(w), _)) = terminal_size::terminal_size() {
        if w > 0 {
            return w as usize;
        }
    }
    80
}

/// Parse a fixed offset string like `"+05:30"` or `"-08:00"` into a `FixedOffset`.
pub fn parse_fixed_offset(s: &str) -> Option<FixedOffset> {
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
    FixedOffset::east_opt(total_seconds)
}

/// Parse a datetime string: replace space with T, if 10 chars add T00:00:00.
/// Returns a NaiveDateTime on success.
pub fn parse_datetime(s: &str) -> Option<NaiveDateTime> {
    let normalized = s.replace(' ', "T");
    let full = if normalized.len() == 10 {
        format!("{}T00:00:00", normalized)
    } else if normalized.len() == 16 {
        // YYYY-MM-DDTHH:MM -> add :00
        format!("{}:00", normalized)
    } else {
        normalized
    };
    NaiveDateTime::parse_from_str(&full, "%Y-%m-%dT%H:%M:%S").ok()
}

/// Compute effective (from, to) date strings from the various date options.
pub fn compute_date_range(
    from_val: Option<String>,
    to_val: Option<String>,
    h5from_val: Option<String>,
    h5to_val: Option<String>,
    w1from_val: Option<String>,
    w1to_val: Option<String>,
) -> (Option<String>, Option<String>) {
    if let Some(ref val) = h5from_val {
        if let Some(dt) = parse_datetime(val) {
            let end = dt + chrono::Duration::hours(5);
            return (
                Some(dt.format("%Y-%m-%dT%H:%M:%S").to_string()),
                Some(end.format("%Y-%m-%dT%H:%M:%S").to_string()),
            );
        }
        return (from_val, to_val);
    }

    if let Some(ref val) = h5to_val {
        if let Some(dt) = parse_datetime(val) {
            let start = dt - chrono::Duration::hours(5);
            return (
                Some(start.format("%Y-%m-%dT%H:%M:%S").to_string()),
                Some(dt.format("%Y-%m-%dT%H:%M:%S").to_string()),
            );
        }
        return (from_val, to_val);
    }

    if let Some(ref val) = w1from_val {
        if let Some(dt) = parse_datetime(val) {
            let end = dt + chrono::Duration::days(7);
            return (
                Some(dt.format("%Y-%m-%dT%H:%M:%S").to_string()),
                Some(end.format("%Y-%m-%dT%H:%M:%S").to_string()),
            );
        }
        return (from_val, to_val);
    }

    if let Some(ref val) = w1to_val {
        if let Some(dt) = parse_datetime(val) {
            let start = dt - chrono::Duration::days(7);
            return (
                Some(start.format("%Y-%m-%dT%H:%M:%S").to_string()),
                Some(dt.format("%Y-%m-%dT%H:%M:%S").to_string()),
            );
        }
        return (from_val, to_val);
    }

    (from_val, to_val)
}

/// Map an output format name to its file extension.
pub fn ext_for_format(fmt: &str) -> &str {
    match fmt {
        "markdown" => "md",
        "json" => "json",
        "html" => "html",
        "txt" => "txt",
        "csv" => "csv",
        "tsv" => "tsv",
        _ => fmt,
    }
}

/// Base64-encode bytes (standard alphabet, with padding).
pub fn base64_encode(input: &[u8]) -> String {
    const CHARS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut result = String::with_capacity(input.len().div_ceil(3) * 4);
    for chunk in input.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = if chunk.len() > 1 { chunk[1] as u32 } else { 0 };
        let b2 = if chunk.len() > 2 { chunk[2] as u32 } else { 0 };
        let triple = (b0 << 16) | (b1 << 8) | b2;
        result.push(CHARS[((triple >> 18) & 0x3F) as usize] as char);
        result.push(CHARS[((triple >> 12) & 0x3F) as usize] as char);
        if chunk.len() > 1 {
            result.push(CHARS[((triple >> 6) & 0x3F) as usize] as char);
        } else {
            result.push('=');
        }
        if chunk.len() > 2 {
            result.push(CHARS[(triple & 0x3F) as usize] as char);
        } else {
            result.push('=');
        }
    }
    result
}

/// Copy text to clipboard via OSC 52 terminal escape sequence.
/// Works in most modern terminals without any external tools.
pub fn osc52_copy(text: &str) -> Result<(), String> {
    let encoded = base64_encode(text.as_bytes());
    // Write OSC 52 to stderr (which is typically the terminal)
    eprint!("\x1b]52;c;{}\x07", encoded);
    Ok(())
}

/// Copy text to the system clipboard.
/// Tries native clipboard tools first, falls back to OSC 52.
pub fn copy_to_clipboard(text: &str) -> Result<(), String> {
    use std::process;

    let result = if cfg!(target_os = "macos") {
        process::Command::new("pbcopy")
            .stdin(process::Stdio::piped())
            .stdout(process::Stdio::null())
            .stderr(process::Stdio::null())
            .spawn()
    } else if cfg!(target_os = "windows") {
        process::Command::new("clip")
            .stdin(process::Stdio::piped())
            .stdout(process::Stdio::null())
            .stderr(process::Stdio::null())
            .spawn()
    } else {
        // Linux: try xclip, xsel, wl-copy in order
        process::Command::new("xclip")
            .args(["-selection", "clipboard"])
            .stdin(process::Stdio::piped())
            .stdout(process::Stdio::null())
            .stderr(process::Stdio::null())
            .spawn()
            .or_else(|_| {
                process::Command::new("xsel")
                    .arg("--clipboard")
                    .stdin(process::Stdio::piped())
                    .stdout(process::Stdio::null())
                    .stderr(process::Stdio::null())
                    .spawn()
            })
            .or_else(|_| {
                process::Command::new("wl-copy")
                    .stdin(process::Stdio::piped())
                    .stdout(process::Stdio::null())
                    .stderr(process::Stdio::null())
                    .spawn()
            })
    };

    match result {
        Ok(mut child) => {
            if let Some(mut stdin) = child.stdin.take() {
                stdin
                    .write_all(text.as_bytes())
                    .map_err(|e| format!("Failed to write to clipboard: {}", e))?;
                drop(stdin); // Close stdin so the child sees EOF
            }
            let status = child
                .wait()
                .map_err(|e| format!("Clipboard process failed: {}", e))?;
            if status.success() {
                Ok(())
            } else {
                // Native tool failed, fall back to OSC 52
                osc52_copy(text)
            }
        }
        Err(_) => {
            // No native tool found, fall back to OSC 52
            osc52_copy(text)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_datetime_date_only() {
        let dt = parse_datetime("2026-03-15").unwrap();
        assert_eq!(
            dt.format("%Y-%m-%d %H:%M:%S").to_string(),
            "2026-03-15 00:00:00"
        );
    }

    #[test]
    fn test_parse_datetime_with_time() {
        let dt = parse_datetime("2026-03-15T14:30:00").unwrap();
        assert_eq!(dt.format("%H:%M").to_string(), "14:30");
    }

    #[test]
    fn test_parse_datetime_space_separator() {
        let dt = parse_datetime("2026-03-15 14:30:00").unwrap();
        assert_eq!(dt.format("%H:%M").to_string(), "14:30");
    }

    #[test]
    fn test_parse_datetime_hhmm_only() {
        let dt = parse_datetime("2026-03-15T14:30").unwrap();
        assert_eq!(dt.format("%H:%M:%S").to_string(), "14:30:00");
    }

    #[test]
    fn test_parse_datetime_invalid() {
        assert!(parse_datetime("not-a-date").is_none());
    }

    #[test]
    fn test_compute_date_range_5h_from() {
        let (from, to) = compute_date_range(
            None,
            None,
            Some("2026-03-15T10:00:00".to_string()),
            None,
            None,
            None,
        );
        assert_eq!(from.unwrap(), "2026-03-15T10:00:00");
        assert_eq!(to.unwrap(), "2026-03-15T15:00:00");
    }

    #[test]
    fn test_compute_date_range_1w_to() {
        let (from, to) = compute_date_range(
            None,
            None,
            None,
            None,
            None,
            Some("2026-03-15T00:00:00".to_string()),
        );
        assert_eq!(to.unwrap(), "2026-03-15T00:00:00");
        assert_eq!(from.unwrap(), "2026-03-08T00:00:00");
    }

    #[test]
    fn test_compute_date_range_passthrough() {
        let (from, to) = compute_date_range(
            Some("2026-03-01".to_string()),
            Some("2026-03-31".to_string()),
            None,
            None,
            None,
            None,
        );
        assert_eq!(from.unwrap(), "2026-03-01");
        assert_eq!(to.unwrap(), "2026-03-31");
    }

    #[test]
    fn test_ext_for_format() {
        assert_eq!(ext_for_format("markdown"), "md");
        assert_eq!(ext_for_format("json"), "json");
        assert_eq!(ext_for_format("html"), "html");
        assert_eq!(ext_for_format("csv"), "csv");
        assert_eq!(ext_for_format("tsv"), "tsv");
        assert_eq!(ext_for_format("txt"), "txt");
        assert_eq!(ext_for_format("unknown"), "unknown");
    }

    #[test]
    fn test_base64_encode() {
        assert_eq!(base64_encode(b"Hello"), "SGVsbG8=");
        assert_eq!(base64_encode(b""), "");
        assert_eq!(base64_encode(b"a"), "YQ==");
        assert_eq!(base64_encode(b"ab"), "YWI=");
    }

    #[test]
    fn test_parse_fixed_offset_positive() {
        let fo = parse_fixed_offset("+05:30").unwrap();
        assert_eq!(fo.local_minus_utc(), 5 * 3600 + 30 * 60);
    }

    #[test]
    fn test_parse_fixed_offset_negative() {
        let fo = parse_fixed_offset("-08:00").unwrap();
        assert_eq!(fo.local_minus_utc(), -(8 * 3600));
    }

    #[test]
    fn test_parse_fixed_offset_invalid() {
        assert!(parse_fixed_offset("UTC").is_none());
        assert!(parse_fixed_offset("").is_none());
        assert!(parse_fixed_offset("abc").is_none());
    }

    #[test]
    fn test_term_width_returns_positive() {
        // term_width() should always return a positive value
        // (from COLUMNS env, terminal size, or fallback 80).
        let w = term_width();
        assert!(w > 0);
    }

    #[test]
    fn test_osc52_copy_succeeds() {
        // osc52_copy writes an escape sequence to stderr and returns Ok(()).
        assert!(osc52_copy("hello world").is_ok());
    }

    #[test]
    fn test_compute_date_range_5h_to() {
        // h5to_val sets the END; start is 5 hours earlier.
        let (from, to) = compute_date_range(
            None,
            None,
            None,
            Some("2026-03-15T15:00:00".to_string()),
            None,
            None,
        );
        assert_eq!(from.unwrap(), "2026-03-15T10:00:00");
        assert_eq!(to.unwrap(), "2026-03-15T15:00:00");
    }

    #[test]
    fn test_compute_date_range_1w_from() {
        // w1from_val sets the START; end is 7 days later.
        let (from, to) = compute_date_range(
            None,
            None,
            None,
            None,
            Some("2026-03-01T00:00:00".to_string()),
            None,
        );
        assert_eq!(from.unwrap(), "2026-03-01T00:00:00");
        assert_eq!(to.unwrap(), "2026-03-08T00:00:00");
    }

    #[test]
    fn test_compute_date_range_invalid_5h_from_fallback() {
        // Invalid date in h5from_val should fall back to from_val / to_val.
        let (from, to) = compute_date_range(
            Some("2026-03-01".to_string()),
            Some("2026-03-31".to_string()),
            Some("not-a-date".to_string()),
            None,
            None,
            None,
        );
        assert_eq!(from.unwrap(), "2026-03-01");
        assert_eq!(to.unwrap(), "2026-03-31");
    }

    #[test]
    fn test_compute_date_range_invalid_5h_to_fallback() {
        let (from, to) = compute_date_range(
            Some("2026-03-01".to_string()),
            Some("2026-03-31".to_string()),
            None,
            Some("bad-date".to_string()),
            None,
            None,
        );
        assert_eq!(from.unwrap(), "2026-03-01");
        assert_eq!(to.unwrap(), "2026-03-31");
    }

    #[test]
    fn test_compute_date_range_invalid_1w_from_fallback() {
        let (from, to) = compute_date_range(
            Some("2026-03-01".to_string()),
            Some("2026-03-31".to_string()),
            None,
            None,
            Some("bad-date".to_string()),
            None,
        );
        assert_eq!(from.unwrap(), "2026-03-01");
        assert_eq!(to.unwrap(), "2026-03-31");
    }

    #[test]
    fn test_compute_date_range_invalid_1w_to_fallback() {
        let (from, to) = compute_date_range(
            Some("2026-03-01".to_string()),
            Some("2026-03-31".to_string()),
            None,
            None,
            None,
            Some("bad-date".to_string()),
        );
        assert_eq!(from.unwrap(), "2026-03-01");
        assert_eq!(to.unwrap(), "2026-03-31");
    }

    #[test]
    fn test_base64_encode_three_bytes() {
        // "abc" is exactly 3 bytes — encodes to 4 chars with no padding.
        assert_eq!(base64_encode(b"abc"), "YWJj");
    }

    #[test]
    fn test_base64_encode_longer() {
        // "Hello, World!" encodes to the standard Base64 value.
        assert_eq!(base64_encode(b"Hello, World!"), "SGVsbG8sIFdvcmxkIQ==");
    }

    #[test]
    fn test_parse_fixed_offset_no_colon() {
        // "+0530" has no colon separator — the function requires one.
        assert!(parse_fixed_offset("+0530").is_none());
    }

    #[test]
    fn test_parse_fixed_offset_utc_offset() {
        let fo = parse_fixed_offset("+00:00").unwrap();
        assert_eq!(fo.local_minus_utc(), 0);
    }
}
