//! Wire types shared by the Reins server and the Reins phone core.
//!
//! Pure data and validation: no IO, no async.

pub mod blob;
pub mod connector;
pub mod desktop;
pub mod device;
pub mod gmail;
pub mod ids;
pub mod join;
pub mod pairing;
pub mod relay;
pub mod remote_mcp;
mod validate;

pub use validate::{ValidationError, check_version, normalize_address};

/// Version stamped on every relay and pairing message (`v` field).
pub const PROTOCOL_VERSION: u32 = 1;

/// The official site (the website, downloads, install scripts), as a literal usable in `concat!`. The Rust apps'
/// built-in defaults derive from it (the desktop's release feed, Autopilot's model downloads), so changing the domain
/// is this line. Elsewhere: `reins.site` in `android/gradle.properties` (Android), `DEFAULT_SITE` in
/// `scripts/install.sh`, `REINS_SITE` in `scripts/release.env.example` (release scripts).
#[macro_export]
macro_rules! official_site {
    () => {
        "https://reins2fa.com"
    };
}

/// [`official_site!`] as a constant.
pub const OFFICIAL_SITE: &str = official_site!();

/// The hosted Reins server, where accounts live, as a literal usable in `concat!`. Every client signs in to it unless
/// told otherwise (self-hosted servers: `reins login <server>`, the phone apps' server field). Elsewhere:
/// `reins.defaultServer` in `android/gradle.properties` (Android), `SignInState.defaultServer` in
/// `ios/Reins/Screens/SignIn/SignInScreen.swift` (iOS).
#[macro_export]
macro_rules! default_server {
    () => {
        "https://app.reins2fa.com"
    };
}

/// [`default_server!`] as a constant.
pub const DEFAULT_SERVER: &str = default_server!();

pub mod account_state;
