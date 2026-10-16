//! Cross-check against Windows' own `ping.exe` (dev builds): parses its
//! reply times and compares them with our ICMP median (they must agree
//! within ±5 ms).
//!
//! Only reply times leave the parser. `ping.exe` output also contains the
//! target address; it is never stored, returned or logged.

use serde::Serialize;

/// Agreement we require with `ping.exe`.
pub const TOLERANCE_MS: f64 = 5.0;

/// Reply times in ms from `ping.exe` output, in order.
///
/// The output is localised ("time=33ms", "Zeit=33ms", "temps=33 ms",
/// "time<1ms"), so this looks for the shape `=` or `<`, digits, optional
/// space, `ms`, rather than for English words. `<1ms` counts as 0.5 ms.
pub fn parse_reply_times(output: &str) -> Vec<f64> {
    let mut times = Vec::new();
    for line in output.lines() {
        // Each reply line has exactly one time; summary lines have several
        // `= Nms` values (min/max/avg) and are skipped by requiring one match.
        let found: Vec<f64> = times_in_line(line);
        if found.len() == 1 {
            times.push(found[0]);
        }
    }
    times
}

fn times_in_line(line: &str) -> Vec<f64> {
    let bytes = line.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        let sign = bytes[i];
        if sign == b'=' || sign == b'<' {
            let mut j = i + 1;
            while j < bytes.len() && bytes[j] == b' ' {
                j += 1;
            }
            let start = j;
            while j < bytes.len() && bytes[j].is_ascii_digit() {
                j += 1;
            }
            let digits = &line[start..j];
            while j < bytes.len() && bytes[j] == b' ' {
                j += 1;
            }
            if !digits.is_empty() && line[j..].starts_with("ms") {
                if let Ok(n) = digits.parse::<f64>() {
                    out.push(if sign == b'<' { n / 2.0 } else { n });
                }
                i = j + 2;
                continue;
            }
        }
        i += 1;
    }
    out
}

fn median(values: &[f64]) -> Option<f64> {
    if values.is_empty() {
        return None;
    }
    let mut v = values.to_vec();
    v.sort_by(f64::total_cmp);
    let n = v.len();
    Some(if n % 2 == 1 {
        v[n / 2]
    } else {
        (v[n / 2 - 1] + v[n / 2]) / 2.0
    })
}

/// Result shown in the dev panel. No address.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Comparison {
    pub efsp_median_ms: Option<f64>,
    pub ping_exe_median_ms: Option<f64>,
    pub ping_exe_replies: u32,
    /// efsp minus ping.exe.
    pub difference_ms: Option<f64>,
    pub within_tolerance: bool,
}

pub fn compare(efsp_median_ms: Option<f64>, ping_exe_output: &str) -> Comparison {
    let times = parse_reply_times(ping_exe_output);
    let ping_exe_median_ms = median(&times);
    let difference_ms = efsp_median_ms
        .zip(ping_exe_median_ms)
        .map(|(a, b)| ((a - b) * 10.0).round() / 10.0);
    Comparison {
        efsp_median_ms,
        ping_exe_median_ms,
        ping_exe_replies: times.len() as u32,
        difference_ms,
        within_tolerance: difference_ms.is_some_and(|d| d.abs() <= TOLERANCE_MS),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Sample outputs use documentation addresses only.
    const ENGLISH: &str = "
Pinging 203.0.113.7 with 32 bytes of data:
Reply from 203.0.113.7: bytes=32 time=33ms TTL=55
Reply from 203.0.113.7: bytes=32 time=35ms TTL=55
Request timed out.
Reply from 203.0.113.7: bytes=32 time=31ms TTL=55

Ping statistics for 203.0.113.7:
    Packets: Sent = 4, Received = 3, Lost = 1 (25% loss),
Approximate round trip times in milli-seconds:
    Minimum = 31ms, Maximum = 35ms, Average = 33ms
";

    const GERMAN: &str = "
Ping wird ausgeführt für 203.0.113.7 mit 32 Bytes Daten:
Antwort von 203.0.113.7: Bytes=32 Zeit=40ms TTL=55
Antwort von 203.0.113.7: Bytes=32 Zeit=42ms TTL=55
    Minimum = 40ms, Maximum = 42ms, Mittelwert = 41ms
";

    const FRENCH: &str = "
Réponse de 203.0.113.7 : octets=32 temps=12 ms TTL=55
Réponse de 203.0.113.7 : octets=32 temps<1ms TTL=55
    Minimum = 0ms, Maximum = 12ms, Moyenne = 6ms
";

    #[test]
    fn parses_english_and_skips_summary_lines() {
        assert_eq!(parse_reply_times(ENGLISH), vec![33.0, 35.0, 31.0]);
    }

    #[test]
    fn parses_other_languages() {
        assert_eq!(parse_reply_times(GERMAN), vec![40.0, 42.0]);
        assert_eq!(parse_reply_times(FRENCH), vec![12.0, 0.5]);
    }

    #[test]
    fn compare_within_and_outside_tolerance() {
        let ok = compare(Some(34.0), ENGLISH);
        assert_eq!(ok.ping_exe_median_ms, Some(33.0));
        assert_eq!(ok.difference_ms, Some(1.0));
        assert_eq!(ok.ping_exe_replies, 3);
        assert!(ok.within_tolerance);

        let off = compare(Some(45.0), ENGLISH);
        assert_eq!(off.difference_ms, Some(12.0));
        assert!(!off.within_tolerance);
    }

    #[test]
    fn no_replies_or_no_efsp_numbers_is_not_a_pass() {
        let none = compare(Some(30.0), "Request timed out.\nRequest timed out.\n");
        assert_eq!(
            (none.ping_exe_median_ms, none.within_tolerance),
            (None, false)
        );
        assert!(!compare(None, ENGLISH).within_tolerance);
    }

    #[test]
    fn comparison_never_contains_the_address() {
        let json = serde_json::to_string(&compare(Some(34.0), ENGLISH)).unwrap();
        assert!(!json.contains("203.0.113.7"));
    }
}
