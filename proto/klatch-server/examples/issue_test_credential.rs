//! A test-only helper (#1123): issue a worker credential for one member in a throwaway store and print its token on
//! stdout, so the Python and TypeScript client suites can run their whole conformance scenario against the real
//! `klatch-server http` (every HTTP connection needs a credential, and `klatch worker issue` writes a key bundle, not
//! a token).
//!
//! Usage: `issue_test_credential MEMBER [CLEARANCE]` with `KLATCH_HOME` and `KLATCH_KEY_DIR` set. It is an example, so
//! it is never in a release archive, a wheel or the npm package, and it refuses to touch anything that is not a
//! throwaway store: the home must be a directory under the system temp directory (or /tmp) and the key directory must
//! be inside the home, so the live `~/.klatch` store and the Keychain are out of its reach by construction.

use std::path::{Path, PathBuf};

/// Resolve symlinks (macOS /tmp is /private/tmp) through the nearest ancestor that exists.
fn canon(p: &Path) -> PathBuf {
    if let Ok(c) = p.canonicalize() {
        return c;
    }
    match (p.parent(), p.file_name()) {
        (Some(parent), Some(name)) => canon(parent).join(name),
        _ => p.to_path_buf(),
    }
}

/// `..` must not climb out: resolve it lexically when the path does not exist yet.
fn lexical(p: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in p.components() {
        match c {
            std::path::Component::ParentDir => {
                out.pop();
            }
            std::path::Component::CurDir => {}
            other => out.push(other),
        }
    }
    out
}

/// Why this store is not a throwaway one, or `None` when it is.
fn refuse(home: &Path, key_dir: &Path, temp_roots: &[PathBuf]) -> Option<String> {
    let home = canon(&lexical(home));
    let key_dir = canon(&lexical(key_dir));
    let roots: Vec<PathBuf> = temp_roots.iter().map(|r| canon(r)).collect();
    if !roots.iter().any(|r| home.starts_with(r) && home != *r) {
        return Some(format!(
            "{} is not a directory under a temp directory: this helper only works in a throwaway store",
            home.display()
        ));
    }
    if !key_dir.starts_with(&home) || key_dir == home {
        return Some(format!(
            "the key directory {} must be inside the throwaway home {}",
            key_dir.display(),
            home.display()
        ));
    }
    None
}

fn main() -> anyhow::Result<()> {
    let mut args = std::env::args().skip(1);
    let member = args
        .next()
        .ok_or_else(|| anyhow::anyhow!("usage: issue_test_credential MEMBER [CLEARANCE]"))?;
    let clearance = args.next().unwrap_or_else(|| "public".to_string());
    let var = |name: &str| {
        std::env::var_os(name)
            .map(PathBuf::from)
            .ok_or_else(|| anyhow::anyhow!("{name} must be set to a throwaway directory"))
    };
    let (home, key_dir) = (var("KLATCH_HOME")?, var("KLATCH_KEY_DIR")?);
    let roots = [std::env::temp_dir(), PathBuf::from("/tmp")];
    if let Some(why) = refuse(&home, &key_dir, &roots) {
        anyhow::bail!("{why}");
    }
    // the identity of this process comes from the environment only: nothing else may redirect the store
    unsafe {
        std::env::remove_var("KLATCH_NAME");
        std::env::remove_var("KLATCH_IDENTITY_FILE");
    }
    let store = klatch::member::custody::open_store(&klatch::member::db_path()?)?;
    let gateway =
        klatch::gateway::Gateway::new(store, klatch::member::CliClock, klatch::gateway::AllowAll);
    let spec = klatch::gateway::CredentialSpec {
        member: &member,
        clearance: &clearance,
        ttl: 3600,
        max_life: None,
        parent: None,
    };
    let token = gateway
        .admin_issue_credential(&klatch::domain::Principal::unattributed("launcher"), &spec)?;
    println!("{}", token.to_text().as_str());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn roots() -> Vec<PathBuf> {
        vec![PathBuf::from("/tmp")]
    }

    #[test]
    fn a_home_and_key_directory_under_the_temp_directory_are_accepted() {
        assert_eq!(
            refuse(
                Path::new("/tmp/kh1/h"),
                Path::new("/tmp/kh1/h/keys"),
                &roots()
            ),
            None
        );
    }

    #[test]
    fn a_home_outside_the_temp_directory_is_refused() {
        let home = std::env::var("HOME").unwrap_or_else(|_| "/Users/nobody".into());
        let live = PathBuf::from(&home).join(".klatch");
        assert!(refuse(&live, &live.join("keys"), &roots()).is_some());
        assert!(
            refuse(
                Path::new("/var/lib/klatch"),
                Path::new("/var/lib/klatch/k"),
                &roots()
            )
            .is_some()
        );
    }

    #[test]
    fn the_temp_directory_itself_is_not_a_throwaway_home() {
        assert!(refuse(Path::new("/tmp"), Path::new("/tmp/keys"), &roots()).is_some());
    }

    #[test]
    fn a_key_directory_outside_the_home_is_refused() {
        assert!(refuse(Path::new("/tmp/kh1/h"), Path::new("/tmp/other"), &roots()).is_some());
        assert!(refuse(Path::new("/tmp/kh1/h"), Path::new("/tmp/kh1/h"), &roots()).is_some());
        assert!(
            refuse(
                Path::new("/tmp/kh1/h"),
                Path::new("/tmp/kh1/h/../../../etc"),
                &roots()
            )
            .is_some()
        );
    }

    #[test]
    fn a_dotdot_in_the_home_cannot_climb_out_of_the_temp_directory() {
        assert!(
            refuse(
                Path::new("/tmp/../etc/h"),
                Path::new("/tmp/../etc/h/keys"),
                &roots()
            )
            .is_some()
        );
    }
}
