//! A policy-managed account cannot hand its credential-owning role to an older app that ignores its policy state.
//! Capability evidence is an independently signed, optional extension of the existing handover request. The
//! primary checks it while holding the same transfer fence used by policy administration.

use reins_proto::authorization::AUTHORIZATION_VERSION;
use reins_proto::roster::{HandoverRequest, HandoverSignature};

use super::{LocalRoster, ed25519_verify};
use crate::{CoreError, engine::Engine};

impl Engine {
    pub(super) fn require_handover_policy_support(
        &self,
        roster: &LocalRoster,
        account: &str,
        request: &HandoverRequest,
    ) -> Result<(), CoreError> {
        // Revoked/paused policies, managed markers and unresolved history remain relevant. Corrupt state is an
        // error rather than a reason to transfer to an app that could ignore the account's restrictions.
        if self.store.authorization_state()?.is_none() {
            return Ok(());
        }
        let member = roster
            .current()
            .viewers
            .iter()
            .find(|m| m.device_id == request.device_id)
            .ok_or_else(|| CoreError::invalid("the receiving phone is not in this account's verified roster"))?;
        let key = member.sign_key().map_err(|e| CoreError::invalid(e.to_string()))?;
        require_policy_support(request.signed.as_ref(), account, &member.device_id, &key)
    }
}

fn require_policy_support(
    signed: Option<&HandoverSignature>,
    account: &str,
    device: &str,
    key: &[u8; 32],
) -> Result<(), CoreError> {
    if signed.is_some_and(|signed| {
        signed.policy_enforcement.as_ref().is_some_and(|p| p.authorization_versions.contains(&AUTHORIZATION_VERSION))
            && signed.policy_support_verifies(account, device, key, ed25519_verify)
    }) {
        return Ok(());
    }
    Err(CoreError::invalid(
        "This account has authorization policies. Update the receiving phone and Reins server, then request the transfer again so its policy support can be verified.",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::roles::keys::SigningKey;
    use reins_proto::roster::{HandoverPolicySupport, handover_policy_support_message, handover_request_message};

    fn signed(versions: Vec<u32>) -> (SigningKey, HandoverSignature) {
        let key = SigningKey::from_seed(&[17; 32]).unwrap();
        let nonce = reins_proto::enroll::encode_nonce(&[7; 32]);
        let message = handover_request_message("acct", 3, "viewer", &nonce, 100).unwrap();
        let proof = handover_policy_support_message("acct", 3, "viewer", &nonce, 100, &versions).unwrap();
        let request = HandoverSignature {
            epoch: 3,
            nonce,
            at: 100,
            sig: key.sign_b64(&message),
            policy_enforcement: Some(HandoverPolicySupport {
                authorization_versions: versions,
                sig: key.sign_b64(&proof),
            }),
        };
        (key, request)
    }

    #[test]
    fn protected_handover_requires_supported_version_and_verified_viewer_claim() {
        let (key, mut request) = signed(vec![AUTHORIZATION_VERSION]);
        assert!(require_policy_support(Some(&request), "acct", "viewer", &key.public()).is_ok());
        assert!(require_policy_support(None, "acct", "viewer", &key.public()).is_err());
        assert!(require_policy_support(Some(&request), "other", "viewer", &key.public()).is_err());
        assert!(require_policy_support(Some(&request), "acct", "other", &key.public()).is_err());
        let other_key = SigningKey::from_seed(&[18; 32]).unwrap();
        assert!(require_policy_support(Some(&request), "acct", "viewer", &other_key.public()).is_err());
        request.policy_enforcement = None;
        assert!(require_policy_support(Some(&request), "acct", "viewer", &key.public()).is_err());
        let (key, request) = signed(vec![AUTHORIZATION_VERSION + 1]);
        assert!(require_policy_support(Some(&request), "acct", "viewer", &key.public()).is_err());
    }
}
