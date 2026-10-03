//! Stamps the release this binary belongs to (set by `scripts/release-desktop.sh`; "dev" and 0 otherwise), so
//! `reins update` can tell whether a published release is newer.

fn main() {
    println!("cargo:rerun-if-env-changed=REINS_BUILD");
    println!("cargo:rerun-if-env-changed=REINS_BUILD_TIME");
    let build = std::env::var("REINS_BUILD")
        .ok()
        .filter(|b| {
            !b.is_empty() && b.len() <= 80 && b.bytes().all(|c| c.is_ascii_alphanumeric() || b".+-_".contains(&c))
        })
        .unwrap_or_else(|| "dev".to_owned());
    let time = std::env::var("REINS_BUILD_TIME").ok().and_then(|t| t.parse::<i64>().ok()).unwrap_or(0);
    println!("cargo:rustc-env=REINS_BUILD={build}");
    println!("cargo:rustc-env=REINS_BUILD_TIME={time}");
}
