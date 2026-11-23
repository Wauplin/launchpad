//! Stamps the companion's version and update channel: CI sets `LAUNCHPAD_DESKTOP_VERSION` (the package version,
//! `YYYY.MM.DD.N`, or `YYYY.MM.DD.0~main.<time>.g<commit>` on main) and `LAUNCHPAD_DESKTOP_CHANNEL` (`release` or
//! `main`); a local build is version `<Cargo version>` on channel `dev`, which checks for no update.
fn main() {
    println!("cargo:rerun-if-env-changed=LAUNCHPAD_DESKTOP_VERSION");
    println!("cargo:rerun-if-env-changed=LAUNCHPAD_DESKTOP_CHANNEL");
    println!("cargo:rerun-if-env-changed=LAUNCHPAD_HF_CLIENT_ID");
    let version = std::env::var("LAUNCHPAD_DESKTOP_VERSION").ok().filter(|v| !v.is_empty()).unwrap_or_else(|| std::env::var("CARGO_PKG_VERSION").unwrap());
    let channel = std::env::var("LAUNCHPAD_DESKTOP_CHANNEL").ok().filter(|c| c == "release" || c == "main").unwrap_or_else(|| "dev".into());
    println!("cargo:rustc-env=LAUNCHPAD_DESKTOP_BUILD_VERSION={version}");
    println!("cargo:rustc-env=LAUNCHPAD_DESKTOP_BUILD_CHANNEL={channel}");
}
