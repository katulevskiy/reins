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
    /// `register_device`: another phone approves for this account, and this one may not take over yet. The app offers
    /// the two ways: the other phone's approval ("add another phone", `join_begin`) or the recovery code (or the
    /// master password) with `unlock_account`; then `register_device` again.
    #[error("{}", reins_proto::device::TAKEOVER_REFUSED)]
    OtherApprovalDevice,
    /// This phone only views the account (phone roles): the change is the primary's to make. Lockdown, stopping a
    /// bypass and revoking a grant (the brake) stay open to it.
    #[error("This phone views the account; decisions are made on the primary phone.")]
    ViewerOnly,
    /// `roster_recover`: the account's history could not be fully checked (`reason`, for the user). Ask whether to
    /// recover anyway, then call it again with `token` as its confirmation: it stands for exactly what the user was
    /// shown, so a recovery that would find something else asks again.
    #[error("{reason}")]
    RecoveryNeedsConfirmation {
        reason: String,
        token: String,
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
