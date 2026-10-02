use thiserror::Error;

/// Every failure the core reports to Kotlin (contracts §D). Messages never
/// contain secrets or URLs.
#[derive(Clone, Debug, PartialEq, Eq, Error, uniffi::Error)]
pub enum CoreError {
    #[error("not signed in")]
    NotLoggedIn,
    #[error("a two-factor code is required")]
    TwoFactorRequired,
    #[error("this account's two-factor method is not supported; use an authenticator app (TOTP)")]
    UnsupportedTwoFactor,
    #[error("wrong email, password or two-factor code")]
    InvalidCredentials,
    #[error("network error: {reason}")]
    Network {
        reason: String,
    },
    #[error("server error {status}: {reason}")]
    Server {
        status: u16,
        reason: String,
    },
    #[error("Gmail access needs your consent")]
    GmailNeedsConsent,
    #[error("Gmail error: {reason}")]
    Gmail {
        reason: String,
    },
    /// An integration besides Gmail failed; the reason is safe to show.
    #[error("{reason}")]
    Service {
        reason: String,
    },
    /// An integration needs the user to sign in or allow something again (a Telegram session that ended).
    #[error("{reason}")]
    ServiceNeedsAttention {
        reason: String,
    },
    #[error("not found")]
    NotFound,
    #[error("invalid input: {reason}")]
    Invalid {
        reason: String,
    },
    #[error("storage error: {reason}")]
    Storage {
        reason: String,
    },
}

impl CoreError {
    pub fn invalid(message: impl Into<String>) -> Self {
        Self::Invalid {
            reason: message.into(),
        }
    }

    pub fn storage(message: impl Into<String>) -> Self {
        Self::Storage {
            reason: message.into(),
        }
    }

    pub fn service(message: impl Into<String>) -> Self {
        Self::Service {
            reason: message.into(),
        }
    }

    pub fn needs_attention(message: impl Into<String>) -> Self {
        Self::ServiceNeedsAttention {
            reason: message.into(),
        }
    }

    pub fn gmail(message: impl Into<String>) -> Self {
        Self::Gmail {
            reason: message.into(),
        }
    }
}

/// Error returned by the Kotlin implementations of the foreign traits.
#[derive(Clone, Debug, PartialEq, Eq, Error, uniffi::Error)]
pub enum ForeignError {
    #[error("user interaction needed")]
    NeedsUserInteraction,
    #[error("{reason}")]
    Failed {
        reason: String,
    },
}

impl From<uniffi::UnexpectedUniFFICallbackError> for ForeignError {
    fn from(e: uniffi::UnexpectedUniFFICallbackError) -> Self {
        Self::Failed {
            reason: e.reason,
        }
    }
}
