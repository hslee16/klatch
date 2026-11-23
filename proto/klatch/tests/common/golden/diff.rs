//! A line diff for a golden that no longer matches: `-` lines are committed, `+` lines are what the run produced.

enum Step {
    Same,
    Del,
    Add,
}

/// The longest-common-subsequence table of the two texts (small inputs only).
fn table(a: &[&str], b: &[&str]) -> Vec<Vec<usize>> {
    let mut t = vec![vec![0usize; b.len() + 1]; a.len() + 1];
    for i in (0..a.len()).rev() {
        for j in (0..b.len()).rev() {
            t[i][j] = if a[i] == b[j] {
                t[i + 1][j + 1] + 1
            } else {
                t[i + 1][j].max(t[i][j + 1])
            };
        }
    }
    t
}

/// What to do at line `i` of the committed text and line `j` of the new one.
fn step(a: &[&str], b: &[&str], t: &[Vec<usize>], at: (usize, usize)) -> Step {
    let (i, j) = at;
    if a.get(i).zip(b.get(j)).is_some_and(|(x, y)| x == y) {
        Step::Same
    } else if j == b.len() {
        Step::Del
    } else if i == a.len() || t[i + 1][j] < t[i][j + 1] {
        Step::Add
    } else {
        Step::Del
    }
}

/// The diff of two texts, one `-` or `+` line (with its line number) per difference; empty when they are the same.
pub fn diff(expected: &str, actual: &str) -> String {
    let a: Vec<&str> = expected.lines().collect();
    let b: Vec<&str> = actual.lines().collect();
    let t = table(&a, &b);
    let (mut i, mut j, mut out) = (0, 0, String::new());
    while i < a.len() || j < b.len() {
        match step(&a, &b, &t, (i, j)) {
            Step::Same => {
                i += 1;
                j += 1;
            }
            Step::Del => {
                out.push_str(&format!("-{:>4} {}\n", i + 1, a[i]));
                i += 1;
            }
            Step::Add => {
                out.push_str(&format!("+{:>4} {}\n", j + 1, b[j]));
                j += 1;
            }
        }
    }
    if expected.ends_with('\n') != actual.ends_with('\n') {
        out.push_str("(the final newline differs)\n");
    }
    out
}
