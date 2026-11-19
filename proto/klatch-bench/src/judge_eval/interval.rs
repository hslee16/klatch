//! Wilson score intervals for the rates in the report (#914). A rate over a few dozen items says little on its own;
//! the interval shows how little, and the report warns when the count is below [`MIN_N`].

/// The z value for a 95 percent two-sided interval.
pub const Z95: f64 = 1.959_963_984_540_054;

/// Below this many items a rate is shown with a warning.
pub const MIN_N: usize = 30;

/// The Wilson 95 percent interval for `successes` of `n`, as fractions in [0, 1]. `None` when `n` is 0.
pub fn wilson95(successes: usize, n: usize) -> Option<(f64, f64)> {
    if n == 0 {
        return None;
    }
    let n_f = n as f64;
    let p = successes.min(n) as f64 / n_f;
    let z2 = Z95 * Z95;
    let denom = 1.0 + z2 / n_f;
    let center = (p + z2 / (2.0 * n_f)) / denom;
    let half = Z95 * (p * (1.0 - p) / n_f + z2 / (4.0 * n_f * n_f)).sqrt() / denom;
    Some(((center - half).max(0.0), (center + half).min(1.0)))
}

/// `12.3% [8.1%, 18.0%] (n=40)`, or `-` when `n` is 0.
pub fn rate_with_interval(successes: usize, n: usize) -> String {
    match wilson95(successes, n) {
        None => format!("- (n={n})"),
        Some((lo, hi)) => format!(
            "{:.1}% [{:.1}%, {:.1}%] (n={n})",
            successes.min(n) as f64 / n as f64 * 100.0,
            lo * 100.0,
            hi * 100.0
        ),
    }
}

#[cfg(test)]
#[path = "interval_tests.rs"]
mod tests;
