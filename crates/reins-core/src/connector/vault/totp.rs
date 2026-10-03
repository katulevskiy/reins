//! One-time codes (RFC 6238) from what a login keeps: a bare base32 secret or an `otpauth://` address.

use std::time::{SystemTime, UNIX_EPOCH};

use data_encoding::BASE32_NOPAD;
use ring::hmac;
use url::Url;
use zeroize::Zeroizing;

use crate::CoreError;

/// A one-time code from what the vault keeps: a bare base32 secret, or an `otpauth://` address. Returns the code
/// and how many seconds it stays valid.
pub fn totp_code(stored: &str, now: u64) -> Result<(String, u64), CoreError> {
    let unsupported = || CoreError::service("That login's one-time code is of a kind Rewarden cannot make.");
    let (secret, digits, period, algorithm) = if stored.trim().to_lowercase().starts_with("otpauth://") {
        let url = Url::parse(stored.trim()).map_err(|_| unsupported())?;
        let get =
            |name: &str| url.query_pairs().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.into_owned());
        (
            get("secret").ok_or_else(unsupported)?,
            get("digits").map_or(Ok(6), |d| d.parse::<u32>()).map_err(|_| unsupported())?,
            get("period").map_or(Ok(30), |p| p.parse::<u64>()).map_err(|_| unsupported())?,
            get("algorithm").unwrap_or_else(|| "SHA1".to_owned()).to_uppercase(),
        )
    } else if stored.trim().to_lowercase().starts_with("steam://") {
        return Err(unsupported());
    } else {
        (stored.to_owned(), 6, 30, "SHA1".to_owned())
    };
    if !(6..=8).contains(&digits) || !(1..=300).contains(&period) {
        return Err(unsupported());
    }
    let cleaned: String = secret.chars().filter(|c| !c.is_whitespace() && *c != '=').collect::<String>().to_uppercase();
    let key_bytes = Zeroizing::new(BASE32_NOPAD.decode(cleaned.as_bytes()).map_err(|_| unsupported())?);
    let algorithm = match algorithm.as_str() {
        "SHA1" => hmac::HMAC_SHA1_FOR_LEGACY_USE_ONLY,
        "SHA256" => hmac::HMAC_SHA256,
        "SHA512" => hmac::HMAC_SHA512,
        _ => return Err(unsupported()),
    };
    let tag = hmac::sign(&hmac::Key::new(algorithm, &key_bytes), &(now / period).to_be_bytes());
    let tag = tag.as_ref();
    let offset = usize::from(tag[tag.len() - 1] & 0x0f);
    let truncated = u32::from_be_bytes([tag[offset] & 0x7f, tag[offset + 1], tag[offset + 2], tag[offset + 3]]);
    let code = truncated % 10u32.pow(digits);
    Ok((format!("{code:0width$}", width = usize::try_from(digits).unwrap_or(6)), period - now % period))
}

/// Whether a one-time code setup can be stored: an `otpauth://` address with a secret, a `steam://` one, or a base32
/// secret. (Storing is wider than [`totp_code`], which cannot make Steam codes.)
pub(super) fn totp_storable(stored: &str) -> bool {
    let lower = stored.trim().to_lowercase();
    if lower.starts_with("otpauth://") {
        return Url::parse(stored.trim())
            .is_ok_and(|u| u.query_pairs().any(|(k, v)| k.eq_ignore_ascii_case("secret") && !v.is_empty()));
    }
    if let Some(rest) = lower.strip_prefix("steam://") {
        return !rest.is_empty();
    }
    let cleaned: String = stored.chars().filter(|c| !c.is_whitespace() && *c != '=').collect::<String>().to_uppercase();
    !cleaned.is_empty() && BASE32_NOPAD.decode(cleaned.as_bytes()).is_ok()
}

pub(super) fn unix_seconds() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

#[cfg(test)]
mod tests {
    use super::*;

    const SHA1_SECRET: &str = "GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ";

    #[test]
    fn one_time_codes_follow_rfc_6238() {
        let eight = format!("otpauth://totp/x?secret={SHA1_SECRET}&digits=8");
        assert_eq!(totp_code(&eight, 59).unwrap(), ("94287082".to_owned(), 1));
        assert_eq!(totp_code(SHA1_SECRET, 59).unwrap().0, "287082", "a bare secret: six digits, thirty seconds");
        assert_eq!(
            totp_code(&format!("otpauth://totp/x?secret={SHA1_SECRET}&digits=8"), 1_111_111_109).unwrap().0,
            "07081804"
        );
        let sha256 = format!(
            "otpauth://totp/x?secret={}&digits=8&algorithm=SHA256",
            BASE32_NOPAD.encode(b"12345678901234567890123456789012")
        );
        assert_eq!(totp_code(&sha256, 59).unwrap().0, "46119246");
        let spaced = "gezd gnbv gy3t qojq gezd gnbv gy3t qojq==";
        assert_eq!(totp_code(spaced, 59).unwrap().0, "287082");
    }

    #[test]
    fn setups_that_can_be_stored() {
        for good in
            [SHA1_SECRET, "gezd gnbv gy3t qojq", "otpauth://totp/x?secret=GEZDGNBV&algorithm=SHA512", "steam://ABCD"]
        {
            assert!(totp_storable(good), "{good}");
        }
        for bad in ["", "otpauth://totp/x", "not base32 1", "steam://"] {
            assert!(!totp_storable(bad), "{bad}");
        }
    }

    #[test]
    fn codes_of_other_kinds_are_refused() {
        for bad in [
            "steam://ABCDEFGH",
            "otpauth://totp/x?secret=!!!",
            "otpauth://totp/x",
            "otpauth://totp/x?secret=GEZDGNBV&digits=12",
            "not base32 1",
        ] {
            assert!(totp_code(bad, 59).is_err(), "{bad}");
        }
    }
}
