//! Wire types shared by the Rewarden server and the Rewarden phone core.
//!
//! Pure data and validation: no IO, no async.

pub mod blob;
pub mod connector;
pub mod desktop;
pub mod device;
pub mod gmail;
pub mod ids;
pub mod pairing;
pub mod relay;
pub mod remote_mcp;
mod validate;

pub use validate::{ValidationError, check_version, normalize_address};

/// Version stamped on every relay and pairing message (`v` field).
pub const PROTOCOL_VERSION: u32 = 1;

/// The official site, as a literal usable in `concat!`. The Rust apps' built-in defaults derive from it (the desktop's
/// release feed, Autopilot's model downloads), so changing the domain is this line. Elsewhere: `rewarden.site` in
/// `android/gradle.properties` (Android), `DEFAULT_SITE` in `scripts/install.sh`, `REWARDEN_SITE` in
/// `scripts/release.env.example` (release scripts).
#[macro_export]
macro_rules! official_site {
    () => {
        "https://rewarden.arc-chat.com"
    };
}

/// [`official_site!`] as a constant.
pub const OFFICIAL_SITE: &str = official_site!();
