use thiserror::Error;

use crate::PROTOCOL_VERSION;

/// Input rejected at the protocol boundary.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum ValidationError {
    #[error("{field}: {reason}")]
    Invalid {
        field: &'static str,
        reason: String,
    },
}

pub(crate) fn invalid(field: &'static str, reason: impl Into<String>) -> ValidationError {
    ValidationError::Invalid {
        field,
        reason: reason.into(),
    }
}

/// Rejects any protocol version (`v` field) other than [`PROTOCOL_VERSION`].
pub fn check_version(v: u32) -> Result<(), ValidationError> {
    if v == PROTOCOL_VERSION {
        Ok(())
    } else {
        Err(invalid("v", format!("unsupported protocol version {v}, expected {PROTOCOL_VERSION}")))
    }
}

/// Validates a bare, ASCII-only email address (`local@domain`) and lower-cases it.
///
/// Display names, whitespace, control characters and header metacharacters are
/// rejected, so a normalized address can be written into an RFC 5322 header
/// without enabling header injection.
pub fn normalize_address(raw: &str) -> Result<String, ValidationError> {
    let bad = |reason: &str| invalid("address", format!("{reason}: {raw:?}"));
    let trimmed = raw.trim();
    // ASCII only (SMTPUTF8/IDN are out of scope). Checked before lowercasing so
    // that e.g. U+212A KELVIN SIGN is rejected rather than folded to `k`.
    if !trimmed.is_ascii() {
        return Err(bad("must be ASCII"));
    }
    let addr = trimmed.to_ascii_lowercase();
    if addr.is_empty() || addr.len() > 254 {
        return Err(bad("length must be 1..=254"));
    }
    if addr.chars().any(|c| c.is_whitespace() || c.is_control() || "<>,;:\"()[]\\".contains(c)) {
        return Err(bad("forbidden character"));
    }
    let Some((local, domain)) = addr.split_once('@') else {
        return Err(bad("missing @"));
    };
    let domain_ok = domain.contains('.')
        && !domain.contains('@')
        && !domain.starts_with('.')
        && !domain.ends_with('.')
        && !domain.contains("..");
    if local.is_empty() || !domain_ok {
        return Err(bad("malformed"));
    }
    Ok(addr)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn check_version_accepts_only_current() {
        assert!(check_version(1).is_ok());
        for bad in [0, 2, u32::MAX] {
            assert!(
                matches!(
                    check_version(bad),
                    Err(ValidationError::Invalid {
                        field: "v",
                        ..
                    })
                ),
                "accepted {bad}"
            );
        }
    }

    #[test]
    fn lowercases_and_trims() {
        assert_eq!(normalize_address("  Alice@Example.COM ").unwrap(), "alice@example.com");
    }

    #[test]
    fn rejects_header_injection_and_display_names() {
        for bad in [
            "a@b.com\r\nBcc: x@evil.com",
            "a@b.com\nx",
            "Bob <bob@x.com>",
            "a@b.com,c@d.com",
            "a b@c.com",
            "\"a\"@b.com",
        ] {
            assert!(normalize_address(bad).is_err(), "accepted {bad:?}");
        }
    }

    #[test]
    fn rejects_non_ascii() {
        for bad in [
            "a@b.com\u{202E}",
            "a\u{202E}b@c.com",
            "a\u{200B}b@c.com",
            "a@b\u{200B}.com",
            "\u{212A}@b.com",
            "a@\u{212A}.com",
            "j\u{f6}rg@b.com",
        ] {
            assert!(normalize_address(bad).is_err(), "accepted {bad:?}");
        }
    }

    #[test]
    fn rejects_malformed() {
        for bad in ["", "a@b", "a@@b.com", "@b.com", "a@.com", "a@b.com.", "a@b..com", "ab.com"] {
            assert!(normalize_address(bad).is_err(), "accepted {bad:?}");
        }
        let long = format!("{}@b.com", "a".repeat(250));
        assert!(normalize_address(&long).is_err());
    }
}
