//! The two durations `ps` prints, as milliseconds: CPU time (`time=`) and elapsed time (`etime=`).

/// `ps -o time=` to milliseconds: `M:SS.cc`, `H:MM:SS` or `D-HH:MM:SS`. `None` for anything else.
pub fn parse_cpu_time(text: &str) -> Option<u64> {
    let (days, clock) = split_days(text)?;
    let (hours, minutes, seconds) = match clock.split(':').collect::<Vec<_>>()[..] {
        [m, s] => (0, whole_number(m)?, s),
        [h, m, s] => (whole_number(h)?, whole_number(m)?, s),
        _ => return None,
    };
    let ms = (decimal(seconds)? * 1000.0).round() as u64;
    Some(((days * 24 + hours) * 60 + minutes) * 60_000 + ms)
}

/// `ps -o etime=` to milliseconds: `MM:SS`, `HH:MM:SS` or `D-HH:MM:SS`, whole numbers only. `None` for anything else.
pub fn parse_elapsed(text: &str) -> Option<u64> {
    let (days, clock) = split_days(text)?;
    let parts = clock
        .split(':')
        .map(whole_number)
        .collect::<Option<Vec<_>>>()?;
    let (hours, minutes, seconds) = match parts[..] {
        [m, s] => (0, m, s),
        [h, m, s] => (h, m, s),
        _ => return None,
    };
    Some((((days * 24 + hours) * 60 + minutes) * 60 + seconds) * 1000)
}

/// A leading `D-` as days, and the clock after it.
fn split_days(text: &str) -> Option<(u64, &str)> {
    match text.split_once('-') {
        Some((d, rest)) => Some((whole_number(d)?, rest)),
        None => Some((0, text)),
    }
}

fn whole_number(text: &str) -> Option<u64> {
    if text.is_empty() || !text.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    text.parse().ok()
}

/// Seconds with an optional fraction, digits and one dot only.
fn decimal(text: &str) -> Option<f64> {
    if text.is_empty() || !text.bytes().all(is_decimal_byte) {
        return None;
    }
    text.parse().ok()
}

fn is_decimal_byte(b: u8) -> bool {
    b.is_ascii_digit() || b == b'.'
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_cpu_time_reads_the_ps_formats() {
        assert_eq!(parse_cpu_time("0:00.78"), Some(780));
        assert_eq!(parse_cpu_time("82:51.93"), Some(82 * 60_000 + 51_930));
        assert_eq!(parse_cpu_time("1:02:03"), Some(3_723_000));
        assert_eq!(
            parse_cpu_time("2-01:00:00"),
            Some(2 * 86_400_000 + 3_600_000)
        );
    }

    #[test]
    fn parse_cpu_time_refuses_everything_else() {
        for bad in ["", "abc", "1:2:3:4", "-", "1:xx"] {
            assert_eq!(parse_cpu_time(bad), None, "{bad}");
        }
    }

    #[test]
    fn elapsed_time_parses_every_ps_shape() {
        assert_eq!(parse_elapsed("05:30"), Some(330_000));
        assert_eq!(parse_elapsed("1:02:03"), Some(3_723_000));
        assert_eq!(parse_elapsed("2-03:04:05"), Some(183_845_000));
        assert_eq!(parse_elapsed("00:00"), Some(0));
    }

    #[test]
    fn elapsed_time_refuses_everything_else() {
        let bad = ["", "-", "5", "a:b", "1:2:3:4", "1-2", "x-01:02:03"];
        for text in bad.into_iter().chain(["01:-2", "00:61x", "1.5:00"]) {
            assert_eq!(parse_elapsed(text), None, "{text:?}");
        }
    }
}
