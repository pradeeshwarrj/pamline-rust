//! Dates and times read like R's `strptime` / `as.Date(x, format)`:
//! numbers may have fewer digits (`5-4-2026`), whitespace before a number is
//! skipped, text after the format is ignored, impossible dates give None.

use chrono::{NaiveDate, NaiveDateTime, NaiveTime, TimeZone};

#[derive(Default)]
struct Parts {
    year: Option<i32>,
    month: Option<u32>,
    day: Option<u32>,
    hour: u32,
    minute: u32,
    second: u32,
}

fn number(s: &[u8], pos: &mut usize, max_digits: usize) -> Option<i64> {
    while *pos < s.len() && s[*pos].is_ascii_whitespace() {
        *pos += 1;
    }
    let start = *pos;
    let neg = *pos < s.len() && (s[*pos] == b'-' || s[*pos] == b'+') && max_digits == 4;
    if neg {
        *pos += 1;
    }
    let digits_start = *pos;
    while *pos < s.len() && *pos - digits_start < max_digits && s[*pos].is_ascii_digit() {
        *pos += 1;
    }
    if *pos == digits_start {
        *pos = start;
        return None;
    }
    let v: i64 = std::str::from_utf8(&s[digits_start..*pos]).ok()?.parse().ok()?;
    Some(if neg && s[start] == b'-' { -v } else { v })
}

const MONTHS: [&str; 12] = ["jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec"];

fn parse(text: &str, format: &str) -> Option<Parts> {
    let s = text.as_bytes();
    let f = format.as_bytes();
    let (mut i, mut p) = (0usize, Parts::default());
    let mut k = 0usize;
    while k < f.len() {
        let c = f[k];
        if c == b'%' && k + 1 < f.len() {
            let spec = f[k + 1];
            k += 2;
            match spec {
                b'Y' => p.year = Some(number(s, &mut i, 4)? as i32),
                b'y' => {
                    let v = number(s, &mut i, 2)? as i32;
                    p.year = Some(if v < 69 { 2000 + v } else { 1900 + v });
                }
                b'm' => p.month = Some(number(s, &mut i, 2)? as u32),
                b'd' | b'e' => p.day = Some(number(s, &mut i, 2)? as u32),
                b'H' => p.hour = number(s, &mut i, 2)? as u32,
                b'M' => p.minute = number(s, &mut i, 2)? as u32,
                b'S' => p.second = number(s, &mut i, 2)? as u32,
                b'b' | b'B' | b'h' => {
                    let rest = text.get(i..)?.to_lowercase();
                    let m = MONTHS.iter().position(|m| rest.starts_with(m))?;
                    // the full name when it is there, else the 3 letters
                    let full = ["january", "february", "march", "april", "may", "june", "july", "august",
                        "september", "october", "november", "december"][m];
                    i += if rest.starts_with(full) { full.len() } else { 3 };
                    p.month = Some(m as u32 + 1);
                }
                b'%' => {
                    if s.get(i) != Some(&b'%') {
                        return None;
                    }
                    i += 1;
                }
                _ => return None,
            }
        } else if c.is_ascii_whitespace() {
            while i < s.len() && s[i].is_ascii_whitespace() {
                i += 1;
            }
            k += 1;
        } else {
            if s.get(i) != Some(&c) {
                return None;
            }
            i += 1;
            k += 1;
        }
    }
    Some(p) // trailing text is ignored, as in R
}

/// `as.Date(text, format = format)`.
pub fn parse_date(text: &str, format: &str) -> Option<NaiveDate> {
    let p = parse(text, format)?;
    NaiveDate::from_ymd_opt(p.year?, p.month?, p.day?)
}

/// `as.POSIXct(text, format = format, tz = tz)`: UTC seconds, None when the
/// text does not fit or the local time does not exist.
pub fn parse_local_time(text: &str, format: &str, tz: &chrono_tz::Tz) -> Option<f64> {
    let p = parse(text, format)?;
    let date = NaiveDate::from_ymd_opt(p.year?, p.month?, p.day?)?;
    if p.hour > 24 || p.minute > 59 || p.second > 61 {
        return None;
    }
    let (h, extra) = if p.hour == 24 { (0, 86_400) } else { (p.hour, 0) };
    let time = NaiveTime::from_hms_opt(h, p.minute, p.second.min(59))?;
    let local = NaiveDateTime::new(date, time);
    let utc = tz.from_local_datetime(&local).earliest()?;
    Some((utc.timestamp() + extra) as f64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates_like_r() {
        assert_eq!(parse_date("05-04-2026", "%d-%m-%Y"), NaiveDate::from_ymd_opt(2026, 4, 5));
        assert_eq!(parse_date("5-4-2026", "%d-%m-%Y"), NaiveDate::from_ymd_opt(2026, 4, 5));
        assert_eq!(parse_date("08-04-0206", "%d-%m-%Y"), NaiveDate::from_ymd_opt(206, 4, 8));
        assert_eq!(parse_date("31-02-2026", "%d-%m-%Y"), None);
        assert_eq!(parse_date("05-04-2026 extra", "%d-%m-%Y"), NaiveDate::from_ymd_opt(2026, 4, 5));
        assert_eq!(parse_date("2026/04/05", "%d-%m-%Y"), None);
        let tz: chrono_tz::Tz = "Asia/Kolkata".parse().unwrap();
        assert_eq!(parse_local_time("2026-04-05 06:00", "%Y-%m-%d %H:%M", &tz), Some(1775349000.0));
        assert_eq!(parse_local_time("2026-04-05 NA", "%Y-%m-%d %H:%M", &tz), None);
    }
}
