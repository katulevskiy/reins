import Foundation

/// Signing with a work profile's enrolled org key (O2): an org review's answer, the member's own policy. The key is the
/// one made at enrollment (the Secure Enclave's when the phone has one), without a user-presence requirement: the
/// phone's lock gates the app.
extension DeviceKeys {
    /// ECDSA P-256 SHA-256 of `message` with `profileId`'s org key: DER, standard base64 (what the org verifies).
    func sign(_ message: Data, profileId: String) throws -> String {
        try sign(profileId: profileId, data: message).base64EncodedString()
    }
}

/// The bytes an org review's answer signs: the core's (`org_review_message`, also `Propose` for your own change).
enum OrgSigning {
    static func message(orgId: String, review: OrgReviewView, approve: Bool, at: Int64) -> Data {
        orgReviewMessage(orgId: orgId, review: review, approve: approve, at: at)
    }
}
