use std::{env, fs, path::PathBuf};

fn main() {
    let pubspec = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap()).join("../../pubspec.yaml");
    println!("cargo:rerun-if-changed={}", pubspec.display());
    println!("cargo:rerun-if-env-changed=THMR_BUILD_VERSION");
    println!("cargo:rerun-if-env-changed=THMR_BUILD_NUMBER");
    let content = fs::read_to_string(pubspec).expect("read unified Flutter product version");
    let raw = content
        .lines()
        .find_map(|line| line.strip_prefix("version:"))
        .expect("pubspec version")
        .trim();
    let (version, build) = raw.split_once('+').unwrap_or((raw, "1"));
    println!(
        "cargo:rustc-env=THMR_VERSION={}",
        env::var("THMR_BUILD_VERSION").unwrap_or(version.into())
    );
    println!(
        "cargo:rustc-env=THMR_BUILD={}",
        env::var("THMR_BUILD_NUMBER").unwrap_or(build.into())
    );
}
