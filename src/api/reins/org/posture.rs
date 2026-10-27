//! Posture gating (`docs/design/decisions-v2/06-trust-and-compliance.md` §6): "A failing device can't approve, and its
//! reviews route to others." The rule is the active org policy's [`PostureRule`]; the posture is what the enrolled
//! device last reported, and the attestation what the server verified at enrollment.

use std::cmp::Ordering;

use reins_policy::org::PostureRule;
use reins_proto::org::Posture;

use crate::db::models::reins_org::{ReinsOrgDevice, attestation};

/// Why a device may not decide.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PostureFail {
    Revoked,
    NoScreenLock,
    Rooted,
    NotAttested,
    OldOs {
        min: String,
    },
    OldApp {
        min: String,
    },
    Stale,
}

impl PostureFail {
    /// Short words for the reviewer's phone and the console.
    #[must_use]
    pub fn text(&self) -> String {
        match self {
            Self::Revoked => "This device was removed by your organization".to_owned(),
            Self::NoScreenLock => "Set a screen lock to approve".to_owned(),
            Self::Rooted => "This device can't approve: its system was modified".to_owned(),
            Self::NotAttested => "This device couldn't prove it's genuine".to_owned(),
            Self::OldOs {
                min,
            } => format!("Update the system to {min} or later to approve"),
            Self::OldApp {
                min,
            } => format!("Update Reins to {min} or later to approve"),
            Self::Stale => "Open Reins to refresh this device's status".to_owned(),
        }
    }
}

/// Compares dotted versions numerically (`17.4 < 17.10`); a part that isn't a number compares as 0.
#[must_use]
pub fn compare_versions(a: &str, b: &str) -> Ordering {
    let parts = |s: &str| -> Vec<u64> {
        s.trim()
            .split(['.', '-', ' '])
            .map(|p| p.chars().take_while(char::is_ascii_digit).collect::<String>().parse().unwrap_or(0))
            .collect()
    };
    let (x, y) = (parts(a), parts(b));
    for i in 0..x.len().max(y.len()) {
        match x.get(i).unwrap_or(&0).cmp(y.get(i).unwrap_or(&0)) {
            Ordering::Equal => {}
            o => return o,
        }
    }
    Ordering::Equal
}

/// Whether `device` may decide under `rule` at `now`.
pub fn may_decide(device: &ReinsOrgDevice, rule: &PostureRule, now: i64) -> Result<(), PostureFail> {
    if device.revoked_at.is_some() {
        return Err(PostureFail::Revoked);
    }
    let posture: Posture = serde_json::from_str(&device.posture).map_err(|_| PostureFail::Stale)?;
    if now - device.posture_at > i64::from(rule.max_age_days) * 86_400 {
        return Err(PostureFail::Stale);
    }
    if rule.screen_lock && !posture.screen_lock {
        return Err(PostureFail::NoScreenLock);
    }
    if rule.no_root && posture.rooted != Some(false) {
        return Err(PostureFail::Rooted);
    }
    if rule.attested && device.attestation_state != attestation::VERIFIED {
        return Err(PostureFail::NotAttested);
    }
    if let Some(min) = rule.min_os.iter().find(|(os, _)| os.eq_ignore_ascii_case(&posture.os_name)).map(|(_, v)| v)
        && compare_versions(&posture.os_version, min) == Ordering::Less
    {
        return Err(PostureFail::OldOs {
            min: min.clone(),
        });
    }
    if let Some(min) = &rule.min_app
        && compare_versions(&posture.app_version, min) == Ordering::Less
    {
        return Err(PostureFail::OldApp {
            min: min.clone(),
        });
    }
    Ok(())
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub fn device(posture: &Posture, attested: &str) -> ReinsOrgDevice {
        ReinsOrgDevice {
            org_uuid: "o1".into(),
            user_uuid: "u1".into(),
            device_uuid: "d1".into(),
            key_algorithm: "p256".into(),
            public_key: String::new(),
            hardware: true,
            attestations: "{}".into(),
            posture: serde_json::to_string(posture).unwrap(),
            posture_at: 1_000,
            enrolled_at: 1_000,
            revoked_at: None,
            attestation_state: attested.into(),
            attestation_detail: None,
        }
    }

    pub fn good() -> Posture {
        Posture {
            screen_lock: true,
            os_name: "iOS".into(),
            os_version: "17.10".into(),
            rooted: Some(false),
            app_version: "1.4.0".into(),
            collected_at: 1_000,
        }
    }

    #[test]
    fn a_good_device_decides() {
        assert_eq!(may_decide(&device(&good(), attestation::NONE), &PostureRule::default(), 2_000), Ok(()));
    }

    #[test]
    fn a_rooted_device_cannot_decide() {
        let mut p = good();
        p.rooted = Some(true);
        assert_eq!(
            may_decide(&device(&p, attestation::VERIFIED), &PostureRule::default(), 2_000),
            Err(PostureFail::Rooted)
        );
    }

    #[test]
    fn unknown_root_status_fails_when_required() {
        let mut p = good();
        p.rooted = None;
        assert_eq!(
            may_decide(&device(&p, attestation::NONE), &PostureRule::default(), 2_000),
            Err(PostureFail::Rooted)
        );
        let lax = PostureRule {
            no_root: false,
            ..PostureRule::default()
        };
        assert_eq!(may_decide(&device(&p, attestation::NONE), &lax, 2_000), Ok(()));
    }

    #[test]
    fn versions_compare_numerically() {
        assert_eq!(compare_versions("17.4", "17.10"), Ordering::Less);
        assert_eq!(compare_versions("17.10", "17.4"), Ordering::Greater);
        assert_eq!(compare_versions("17", "17.0.0"), Ordering::Equal);
        assert_eq!(compare_versions("16 (beta)", "16"), Ordering::Equal);
        let rule = PostureRule {
            min_os: [("ios".to_owned(), "17.5".to_owned())].into(),
            min_app: Some("1.5".into()),
            ..PostureRule::default()
        };
        assert_eq!(
            may_decide(&device(&good(), attestation::NONE), &rule, 2_000),
            Err(PostureFail::OldApp {
                min: "1.5".into()
            })
        );
        let mut p = good();
        p.os_version = "17.4.1".into();
        assert!(matches!(may_decide(&device(&p, attestation::NONE), &rule, 2_000), Err(PostureFail::OldOs { .. })));
    }

    #[test]
    fn an_unattested_device_fails_only_when_attestation_is_required() {
        let rule = PostureRule {
            attested: true,
            ..PostureRule::default()
        };
        assert_eq!(may_decide(&device(&good(), attestation::FAILED), &rule, 2_000), Err(PostureFail::NotAttested));
        assert_eq!(may_decide(&device(&good(), attestation::VERIFIED), &rule, 2_000), Ok(()));
        assert_eq!(may_decide(&device(&good(), attestation::FAILED), &PostureRule::default(), 2_000), Ok(()));
    }

    #[test]
    fn a_stale_posture_or_a_revoked_device_fails() {
        let d = device(&good(), attestation::NONE);
        assert_eq!(may_decide(&d, &PostureRule::default(), 1_000 + 31 * 86_400), Err(PostureFail::Stale));
        let mut revoked = d;
        revoked.revoked_at = Some(5);
        assert_eq!(may_decide(&revoked, &PostureRule::default(), 2_000), Err(PostureFail::Revoked));
        let mut no_lock = good();
        no_lock.screen_lock = false;
        assert_eq!(
            may_decide(&device(&no_lock, attestation::NONE), &PostureRule::default(), 2_000),
            Err(PostureFail::NoScreenLock)
        );
    }
}
