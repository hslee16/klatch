//! A line diff shown before a settings file is written (#1502).

/// The longest-common-subsequence table of two line lists: `table[i][j]` is the length for `a[i..]` and `b[j..]`.
pub(super) fn lcs_table(a: &[&str], b: &[&str]) -> Vec<Vec<usize>> {
    let mut table = vec![vec![0usize; b.len() + 1]; a.len() + 1];
    for i in (0..a.len()).rev() {
        for j in (0..b.len()).rev() {
            table[i][j] = if a[i] == b[j] {
                table[i + 1][j + 1] + 1
            } else {
                table[i + 1][j].max(table[i][j + 1])
            };
        }
    }
    table
}

/// What the next line of a diff is.
pub(super) enum Step {
    Same,
    Add,
    Remove,
}

/// The next step through two line lists at positions `i` and `j`, following the longest common subsequence.
pub(super) fn next_step(
    a: &[&str],
    b: &[&str],
    lcs: &[Vec<usize>],
    (i, j): (usize, usize),
) -> Step {
    match (a.get(i), b.get(j)) {
        (Some(x), Some(y)) if x == y => Step::Same,
        (Some(_), None) => Step::Remove,
        (None, _) => Step::Add,
        (Some(_), Some(_)) if lcs[i][j + 1] >= lcs[i + 1][j] => Step::Add,
        (Some(_), Some(_)) => Step::Remove,
    }
}

/// A line diff of two texts: removed lines with `-`, added with `+`, unchanged lines with two spaces.
pub fn diff(old: &str, new: &str) -> String {
    let a: Vec<&str> = old.lines().collect();
    let b: Vec<&str> = new.lines().collect();
    let lcs = lcs_table(&a, &b);
    let (mut i, mut j, mut out) = (0, 0, String::new());
    while i < a.len() || j < b.len() {
        match next_step(&a, &b, &lcs, (i, j)) {
            Step::Same => {
                out.push_str(&format!("  {}\n", a[i]));
                i += 1;
                j += 1;
            }
            Step::Add => {
                out.push_str(&format!("+ {}\n", b[j]));
                j += 1;
            }
            Step::Remove => {
                out.push_str(&format!("- {}\n", a[i]));
                i += 1;
            }
        }
    }
    out
}
