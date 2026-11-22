//! cargo-binstall (#429) downloads a prebuilt archive instead of compiling. Its metadata in Cargo.toml must name the
//! same archive the release workflow builds, or `cargo binstall klatch` would 404 and fall back to compiling.

use std::path::Path;

fn read(rel: &str) -> String {
    std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join(rel))
        .unwrap_or_else(|e| panic!("{rel}: {e}"))
}

fn metadata_value(manifest: &str, key: &str) -> String {
    let in_table = manifest
        .split("[package.metadata.binstall]")
        .nth(1)
        .expect("a [package.metadata.binstall] table");
    let table = in_table.split("\n[").next().unwrap();
    let line = table
        .lines()
        .find(|l| l.trim_start().starts_with(key))
        .unwrap_or_else(|| panic!("binstall key {key}"));
    line.split_once('=')
        .unwrap()
        .1
        .trim()
        .trim_matches('"')
        .to_string()
}

#[test]
fn the_binstall_url_is_the_release_archive() {
    let manifest = read("Cargo.toml");
    let url = metadata_value(&manifest, "pkg-url");
    assert_eq!(
        url,
        "{ repo }/releases/download/v{ version }/klatch-{ version }-{ target }.tar.gz"
    );
    // the workflow builds exactly that name for the same four targets
    let release = read("../../.github/workflows/build-release.yml");
    assert!(
        release.contains(r#"name="klatch-${VERSION}-${TARGET}""#),
        "build-release.yml no longer names archives klatch-<version>-<target>"
    );
    for target in [
        "aarch64-apple-darwin",
        "x86_64-apple-darwin",
        "x86_64-unknown-linux-gnu",
        "aarch64-unknown-linux-gnu",
        "x86_64-unknown-linux-musl",
        "aarch64-unknown-linux-musl",
    ] {
        assert!(release.contains(&format!("target: {target}")), "{target}");
    }
}

#[test]
fn the_binary_sits_in_the_archives_top_directory() {
    let manifest = read("Cargo.toml");
    assert_eq!(metadata_value(&manifest, "pkg-fmt"), "tgz");
    assert_eq!(
        metadata_value(&manifest, "bin-dir"),
        "klatch-{ version }-{ target }/{ bin }{ binary-ext }"
    );
    // the archive is made by release/package-archive.sh (#1236): klatch (and klatch-server) under klatch-<version>-<target>/
    let package = read("../../release/package-archive.sh");
    assert!(
        package.contains(r#"name="klatch-$version-$target""#)
            && package.contains(r#"install -m 0755 "$bin_dir/klatch" "$stage/$name/klatch""#),
        "the archive layout changed"
    );
}
