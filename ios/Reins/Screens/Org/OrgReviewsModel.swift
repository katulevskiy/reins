import Foundation

/// What the work reviews ask of a work profile's core (the real `ReinsCore` through `CoreOrgReviews`; a fake in tests).
protocol OrgReviewsCore: Sendable {
    func orgMembership() async throws -> OrgView?
    func orgReviews() async throws -> [OrgReviewView]
    func orgDecide(id: String, approve: Bool, at: Int64, signature: String) async throws
    func orgPasskeys() async throws -> [OrgPasskeyView]
    func orgConfirmPasskey(id: String, approve: Bool, at: Int64, signature: String) async throws
}

/// The app's core, as `OrgReviewsCore`.
struct CoreOrgReviews: OrgReviewsCore, @unchecked Sendable {
    let core: any ReinsCoreProtocol

    func orgMembership() async throws -> OrgView? { try await core.orgMembership() }
    func orgReviews() async throws -> [OrgReviewView] { try await core.orgReviews() }
    func orgDecide(id: String, approve: Bool, at: Int64, signature: String) async throws {
        try await core.orgDecide(id: id, approve: approve, at: at, signature: signature)
    }
    func orgPasskeys() async throws -> [OrgPasskeyView] { try await core.orgPasskeys() }
    func orgConfirmPasskey(id: String, approve: Bool, at: Int64, signature: String) async throws {
        try await core.orgConfirmPasskey(id: id, approve: approve, at: at, signature: signature)
    }
}

/// A work profile's org reviews (O2): the core lists them, checked and described on the phone; an answer is signed
/// with the key this phone enrolled to the org, over the core's review message, then sent. A web passkey the member
/// added in the browser waits here too, until this phone confirms it (O3). The Personal profile has none and asks
/// nothing.
@MainActor
@Observable
final class OrgReviewsModel {
    /// nil while loading.
    private(set) var reviews: [OrgReviewView]?
    /// The web passkeys that wait for this phone's confirmation.
    private(set) var passkeys: [OrgPasskeyView] = []
    /// The review (or passkey) whose answer is on its way.
    private(set) var busy: String?
    private(set) var error: String?

    private let core: OrgReviewsCore
    private let profile: WorkProfiles.Active
    private let keys: DeviceKeys
    private let message: (String, OrgReviewView, Bool, Int64) -> Data
    private let passkeyMessage: (String, OrgPasskeyView, Int64) -> Data
    private let now: () -> Int64
    private var orgId: String?

    init(
        core: OrgReviewsCore,
        profile: WorkProfiles.Active = WorkProfiles.active,
        keys: DeviceKeys = EnclaveDeviceKeys(),
        message: @escaping (String, OrgReviewView, Bool, Int64) -> Data = { OrgSigning.message(orgId: $0, review: $1, approve: $2, at: $3) },
        passkeyMessage: @escaping (String, OrgPasskeyView, Int64) -> Data = { OrgSigning.passkeyMessage(orgId: $0, key: $1, at: $2) },
        now: @escaping () -> Int64 = { Int64(Date().timeIntervalSince1970) }
    ) {
        self.core = core
        self.profile = profile
        self.keys = keys
        self.message = message
        self.passkeyMessage = passkeyMessage
        self.now = now
    }

    func load() async {
        guard profile.managed else {
            reviews = []
            return
        }
        do {
            reviews = try await core.orgReviews()
            passkeys = await waitingPasskeys()
            error = nil
        } catch {
            if reviews == nil { reviews = [] }
            self.error = error.userMessage
        }
    }

    /// The member's web passkeys not yet confirmed; none when the server has no web approvals.
    private func waitingPasskeys() async -> [OrgPasskeyView] {
        do {
            return try await core.orgPasskeys().filter { !$0.confirmed }
        } catch {
            return []
        }
    }

    private func orgOfProfile() async throws -> String {
        if let orgId { return orgId }
        guard let id = try await core.orgMembership()?.orgId else { throw CoreError.NotFound }
        orgId = id
        return id
    }

    /// Confirm (this passkey is the member's own) or refuse `key`, signed with the enrolled key over the core's message
    /// of the key hash and registration time the card shows the fingerprint of. Refusing revokes it.
    func confirmPasskey(_ key: OrgPasskeyView, approve: Bool) async {
        guard profile.managed, busy == nil else { return }
        busy = key.credentialId
        error = nil
        defer { busy = nil }
        do {
            let org = try await orgOfProfile()
            let at = now()
            let signature = try keys.sign(passkeyMessage(org, key, at), profileId: profile.id)
            try await core.orgConfirmPasskey(id: key.credentialId, approve: approve, at: at, signature: signature)
            passkeys.removeAll { $0.credentialId == key.credentialId }
        } catch CoreError.NotFound {
            passkeys.removeAll { $0.credentialId == key.credentialId }
        } catch let error as CoreError {
            self.error = error.userMessage
        } catch {
            self.error = "This phone can't sign for your organization."
        }
    }

    /// Approve (or, for your own change, confirm) or deny `review`.
    func answer(_ review: OrgReviewView, approve: Bool) async {
        guard profile.managed, busy == nil else { return }
        busy = review.id
        error = nil
        defer { busy = nil }
        do {
            let org = try await orgOfProfile()
            let at = now()
            let signature = try keys.sign(message(org, review, approve, at), profileId: profile.id)
            try await core.orgDecide(id: review.id, approve: approve, at: at, signature: signature)
            reviews?.removeAll { $0.id == review.id }
        } catch CoreError.NotFound {
            // Answered elsewhere, expired, or no longer checks out: it is gone.
            reviews?.removeAll { $0.id == review.id }
        } catch let error as CoreError {
            self.error = error.userMessage
        } catch {
            // The key is gone: this phone is no longer enrolled.
            self.error = "This phone can't sign for your organization."
        }
    }
}
