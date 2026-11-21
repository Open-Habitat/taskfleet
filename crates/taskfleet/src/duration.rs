//! Human-written duration parsing (`30s`, `5m`, `2h 30m`, `7d`).
//!
//! This replaced the `humantime` crate when RUSTSEC-2026-0542 marked it
//! unmaintained. The grammar is the subset of humantime's that Taskfleet's
//! inputs (`run wait --timeout`, `--poll-interval`, `--check-in`, and
//! `tmux.completed_window_ttl`) actually advertise: one or more
//! `<integer><unit>` terms, optionally separated by whitespace, with
//! humantime's unit spellings. `jiff`'s friendly parser was not used because it
//! rejects day and week units for a plain duration, and `7d` is a documented
//! retention value.
//!
//! Months and years keep humantime's fixed lengths (30.44 and 365.25 days) so a
//! value that parsed before still parses to the same duration. Fractions,
//! signs, and unit-less numbers are rejected; callers that accept a bare
//! integer handle it before calling [`parse`].

use std::time::Duration;

/// Seconds and nanoseconds a unit spelling stands for, or `None` if unknown.
fn unit(name: &str) -> Option<(u64, u64)> {
    Some(match name {
        "nsec" | "ns" => (0, 1),
        "usec" | "us" | "µs" => (0, 1_000),
        "msec" | "ms" => (0, 1_000_000),
        "seconds" | "second" | "secs" | "sec" | "s" => (1, 0),
        "minutes" | "minute" | "min" | "mins" | "m" => (60, 0),
        "hours" | "hour" | "hr" | "hrs" | "h" => (3_600, 0),
        "days" | "day" | "d" => (86_400, 0),
        "weeks" | "week" | "w" => (604_800, 0),
        "months" | "month" | "M" => (2_630_016, 0),
        "years" | "year" | "y" => (31_557_600, 0),
        _ => return None,
    })
}

/// Parse a human-written duration. The error is a lower-case sentence fit to
/// follow `invalid duration '<input>': `.
pub(crate) fn parse(input: &str) -> Result<Duration, String> {
    let s = input.trim();
    if s.is_empty() {
        return Err("value was empty".to_string());
    }
    let mut total = Duration::ZERO;
    let mut rest = s;
    while !rest.is_empty() {
        let digits_end = rest
            .find(|c: char| !c.is_ascii_digit())
            .unwrap_or(rest.len());
        if digits_end == 0 {
            return Err(format!(
                "expected a number at {rest:?}, for example 2s or 15m"
            ));
        }
        let number: u64 = rest[..digits_end]
            .parse()
            .map_err(|_| "number is too large".to_string())?;
        rest = rest[digits_end..].trim_start();
        let unit_end = rest
            .find(|c: char| c.is_ascii_digit() || c.is_whitespace())
            .unwrap_or(rest.len());
        let name = &rest[..unit_end];
        if name.is_empty() {
            return Err("time unit needed, for example 2s or 15m".to_string());
        }
        let (secs, nanos) = unit(name).ok_or_else(|| {
            format!("unknown time unit {name:?}, supported units: ns, us, ms, s, m, h, d, w, M, y")
        })?;
        let term = if nanos == 0 {
            number.checked_mul(secs).map(Duration::from_secs)
        } else {
            number.checked_mul(nanos).map(Duration::from_nanos)
        };
        total = term
            .and_then(|t| total.checked_add(t))
            .ok_or_else(|| "number is too large".to_string())?;
        rest = rest[unit_end..].trim_start();
    }
    Ok(total)
}

#[cfg(test)]
mod tests {
    use super::parse;
    use std::time::Duration;

    #[test]
    fn accepts_humantime_spellings() {
        let s = Duration::from_secs;
        for (input, want) in [
            ("30s", s(30)),
            ("5m", s(300)),
            ("1h", s(3_600)),
            ("2h 30m", s(9_000)),
            ("2h30m", s(9_000)),
            ("1hour 15min", s(4_500)),
            ("  24h  ", s(86_400)),
            ("7d", s(604_800)),
            ("365d", s(31_536_000)),
            ("2 weeks", s(1_209_600)),
            ("1M", s(2_630_016)),
            ("1y", s(31_557_600)),
            ("0s", Duration::ZERO),
            ("500ms", Duration::from_millis(500)),
            ("3us", Duration::from_micros(3)),
            ("7ns", Duration::from_nanos(7)),
            ("1s 500ms", Duration::from_millis(1_500)),
        ] {
            assert_eq!(parse(input), Ok(want), "{input:?}");
        }
    }

    #[test]
    fn rejects_malformed_values() {
        for input in [
            "",
            "   ",
            "10",
            "s",
            "-5s",
            "+5s",
            "1.5h",
            "5x",
            "bogus",
            "5 s x",
            "99999999999999999999s",
            "18446744073709551615h",
        ] {
            assert!(parse(input).is_err(), "{input:?} should be rejected");
        }
    }
}
