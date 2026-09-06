//! Timestamp handling without `chrono`: parse RFC3339 (UTC `…Z`) by hand and
//! convert to the system-local date via `libc::localtime_r`.

/// Days from the civil date to the Unix epoch (Howard Hinnant's algorithm).
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

fn local_ymd(secs: i64) -> (i64, u32, u32) {
    let t = secs as libc::time_t;
    let mut tmv: libc::tm = unsafe { std::mem::zeroed() };
    let res = unsafe { libc::localtime_r(&t, &mut tmv) };
    if res.is_null() {
        let (y, m, d) = civil_from_days(secs.div_euclid(86400));
        return (y, m, d);
    }
    (
        tmv.tm_year as i64 + 1900,
        (tmv.tm_mon + 1) as u32,
        tmv.tm_mday as u32,
    )
}

/// Parse an RFC3339/`Z` timestamp → (epoch ms, local YYYY-MM-DD, local YYYY-MM).
pub fn parse_ts(ts: &str) -> Option<(i64, String, String)> {
    let b = ts.as_bytes();
    if b.len() < 19 {
        return None;
    }
    let n = |s: usize, e: usize| ts.get(s..e).and_then(|x| x.parse::<i64>().ok());
    let (y, mo, d) = (n(0, 4)?, n(5, 7)?, n(8, 10)?);
    let (h, mi, s) = (n(11, 13)?, n(14, 16)?, n(17, 19)?);

    // Milliseconds: the first three fraction digits, right-padded with zeros.
    let mut frac = 0i64;
    if b.get(19) == Some(&b'.') {
        for k in 20..23 {
            let digit = b
                .get(k)
                .filter(|c| c.is_ascii_digit())
                .map_or(0, |c| (c - b'0') as i64);
            frac = frac * 10 + digit;
        }
    }

    let secs = days_from_civil(y, mo, d) * 86400 + h * 3600 + mi * 60 + s;
    let ms = secs * 1000 + frac;
    let (ly, lm, ld) = local_ymd(secs);
    Some((
        ms,
        format!("{ly:04}-{lm:02}-{ld:02}"),
        format!("{ly:04}-{lm:02}"),
    ))
}

/// Format epoch milliseconds as a UTC RFC3339 string (matches JS toISOString).
pub fn iso_from_ms(ms: i64) -> String {
    let secs = ms.div_euclid(1000);
    let millis = ms.rem_euclid(1000);
    let days = secs.div_euclid(86400);
    let rem = secs.rem_euclid(86400);
    let (y, m, d) = civil_from_days(days);
    let (h, mi, s) = (rem / 3600, (rem % 3600) / 60, rem % 60);
    format!("{y:04}-{m:02}-{d:02}T{h:02}:{mi:02}:{s:02}.{millis:03}Z")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fraction_digits_become_milliseconds() {
        // The ms value drives the auto-review cutover, session ordering and
        // the Codex replay-burst gap; a wrong scale shifts all three.
        let base = parse_ts("2026-01-01T00:00:00Z").unwrap().0;
        assert_eq!(parse_ts("2026-01-01T00:00:00.5Z").unwrap().0 - base, 500);
        assert_eq!(
            parse_ts("2026-01-01T00:00:00.123456Z").unwrap().0 - base,
            123
        );
        assert_eq!(parse_ts("2026-01-01T00:00:00.000Z").unwrap().0 - base, 0);
    }

    #[test]
    fn iso_round_trips() {
        let ms = 1_785_369_600_123; // 2026-07-30T00:00:00.123Z
        assert_eq!(iso_from_ms(ms), "2026-07-30T00:00:00.123Z");
        assert_eq!(parse_ts(&iso_from_ms(ms)).unwrap().0, ms);
    }
}
