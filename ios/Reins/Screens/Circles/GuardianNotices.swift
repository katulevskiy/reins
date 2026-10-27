import Observation
import SwiftUI

/// The guardian's side of a guardianship (decisions v2 P7, 04 §4), on Settings › Circles: a ward's request to leave
/// that only this account's consent ends ("Sam asked to leave. Allow?"), and what this account was told about its
/// wards, newest first. What this phone checked (a signed change, the roster chain) shows as it is; the rest is
/// labelled "Reported by the server" (it can drop or delay notices, never forge a checked one).
@Observable
@MainActor
final class GuardianNoticesModel {
    private(set) var notices: [GuardianNoticeView] = []
    private(set) var busy = false
    var error: String?

    func load(_ app: AppModel, fromServer: Bool) async {
        let epoch = app.accountEpoch
        do {
            let list = try await app.core.guardianNotices(refresh: fromServer)
            guard epoch == app.accountEpoch else { return }
            notices = GuardianLogic.notices(list)
            error = nil
        } catch {
            guard epoch == app.accountEpoch else { return }
            self.error = error.userMessage
        }
    }

    /// Lets a minor's request to leave go through: it ends now. The core asks for Face ID, Touch ID or the passcode
    /// itself before it signs (`app.ownerCheck`), so no screen can skip it; a cancel consents to nothing.
    func allow(_ ward: CirclePerson, _ app: AppModel) async {
        guard !busy, GuardianLogic.isGuardian(ward), ward.needsConsent else { return }
        await run(app, cue: .approved) { try await app.core.guardianConsentEnd(linkId: ward.linkId, owner: app.ownerCheck) }
    }

    /// Asks the ward to accept marking their account a minor's (nothing changes until they do).
    func proposeMinor(_ ward: CirclePerson, _ app: AppModel) async {
        guard GuardianLogic.canProposeMinor(ward) else { return }
        await run(app, cue: .approved) { try await app.core.guardianProposeMinor(linkId: ward.linkId) }
        if error == nil { app.notice = "Sent to \(CircleLogic.name(ward.peer))." }
    }

    private func run(_ app: AppModel, cue: FeedbackEvent, _ body: @escaping () async throws -> Void) async {
        guard !busy else { return }
        busy = true
        error = nil
        defer { busy = false }
        var failure: String?
        do {
            try await body()
            app.feedback.play(cue)
        } catch {
            app.feedback.play(.error)
            failure = error.userMessage
        }
        // Read again either way; what went wrong stays shown (the read clears it).
        await load(app, fromServer: false)
        if let failure { error = failure }
        await app.circlesChanged()
    }
}

/// "Sam asked to leave. Allow?" with Allow: a minor's request to leave waits for this account alone.
struct GuardianConsentCard: View {
    var ward: CirclePerson
    var busy: Bool
    var onAllow: () -> Void

    var body: some View {
        HStack(spacing: 12) {
            BlobAvatar(seed: untrusted(ward.peer), size: 36)
            Text(verbatim: GuardianLogic.consentLine(ward.peer))
                .textRole(.row)
                .foregroundStyle(Palette.ink)
                .fixedSize(horizontal: false, vertical: true)
                .frame(maxWidth: .infinity, alignment: .leading)
            Button(action: onAllow) {
                HStack(spacing: 6) {
                    if busy { ProgressView() } else { BiometryMark(size: 16) }
                    Text("Allow")
                }
            }
            .buttonStyle(SignalButtonStyle(.inverse, size: .small, fill: false))
            .disabled(busy)
            .accessibilityIdentifier("guardianAllow:\(ward.linkId)")
        }
        .padding(14)
        .background(Palette.violet.opacity(0.08), in: RoundedRectangle(cornerRadius: 18, style: .continuous))
        .padding(.horizontal, 16)
        .padding(.top, 12)
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("guardianConsent:\(ward.linkId)")
    }
}

/// What this account was told about its wards: "Sam · Ended early", with "Reported by the server" under what this
/// phone could not check.
struct GuardianNoticeList: View {
    var notices: [GuardianNoticeView]

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            SectionHeader("About your wards", quiet: true) {
                HelpButton(text: "Guardianship is cooperative: a ward's phone holds to it, and you're told when it changes or ends early.")
            }
            ForEach(notices, id: \.id) { n in
                HStack(alignment: .firstTextBaseline, spacing: 10) {
                    Image(systemName: n.verified ? "checkmark.shield" : "exclamationmark.shield")
                        .font(.system(size: 12, weight: .semibold))
                        .foregroundStyle(n.verified ? Palette.inkMeta : Palette.warning)
                    VStack(alignment: .leading, spacing: 2) {
                        Text(verbatim: "\(CircleLogic.name(n.ward)) · \(GuardianLogic.noticeText(n))")
                            .font(RFont.sans(14))
                            .foregroundStyle(Palette.ink2)
                            .fixedSize(horizontal: false, vertical: true)
                        if let source = GuardianLogic.source(n) {
                            Text(source).font(RFont.sans(12.5)).foregroundStyle(Palette.inkMeta)
                                .accessibilityIdentifier("guardianNoticeSource")
                        }
                    }
                    .frame(maxWidth: .infinity, alignment: .leading)
                    Text(TimeText.relative(n.at)).font(RFont.sans(12.5)).foregroundStyle(Palette.inkMeta)
                }
                .padding(.horizontal, 24)
                .padding(.vertical, 6)
                .accessibilityElement(children: .combine)
                .accessibilityIdentifier("guardianNotice:\(n.id)")
            }
        }
        .accessibilityIdentifier("guardianNotices")
    }
}
