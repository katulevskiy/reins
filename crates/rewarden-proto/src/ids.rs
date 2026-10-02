use std::fmt;

use serde::{Deserialize, Serialize};

macro_rules! id_type {
    ($($(#[$meta:meta])* $name:ident),+ $(,)?) => {$(
        $(#[$meta])*
        #[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(pub String);

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.0)
            }
        }

        impl From<&str> for $name {
            fn from(s: &str) -> Self {
                Self(s.to_owned())
            }
        }
    )+};
}

id_type!(
    /// A relayed tool call waiting for the phone.
    RequestId,
    /// An AI client authorized by a user (one per OAuth authorization).
    ConnectionId,
    /// An in-progress AI-connection pairing.
    PairingId,
    /// A standing permission stored on the phone.
    GrantId,
);
