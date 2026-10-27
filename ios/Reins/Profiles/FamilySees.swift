import Foundation
import SwiftUI

/// The family's glyph (SF Symbols): on the badge, the profile rows and the family's own page.
enum FamilySymbol {
    static let name = "figure.2.and.child.holdinghands"
}

/// What a family profile's core does while it signs in and joins (the real `ReinsCore`; a fake in tests).
protocol FamilySignInCore: AnyObject, Sendable {
    func ssoBegin(serverUrl: String) async throws -> SsoStart
    func ssoFinish(serverUrl: String, callbackUrl: String, state: String, verifier: String) async throws -> SsoOutcome
    func familyJoin(link: String) async throws -> FamilyView
    func registerDevice(fcmToken: String?) async throws
    func pushTag() async throws -> String?
}

extension ReinsCore: FamilySignInCore {}

/// "Add family account" (P7, 08 R6, R7): the invitation link a guardian sent, then a sign-in with a **new** account on
/// its own reserved profile and core, which joins the family. The Personal account never takes part; an account that
/// isn't new is refused by the server, and the reserved profile removed.
@MainActor
@Observable
final class AddFamilyAccount {
    enum Step: Equatable {
        case link
        case working
        case added(label: String, id: String)
    }

    private(set) var step: Step = .link
    private(set) var error: String?
    let registry: ProfileRegistryProtocol
    private let server: String
    private let makeCore: (AccountProfile) throws -> FamilySignInCore
    private let browse: (URL, String) async throws -> URL

    init(
        server: String,
        registry: ProfileRegistryProtocol = WorkProfiles.registry(),
        makeCore: @escaping (AccountProfile) throws -> FamilySignInCore,
        browse: @escaping (URL, String) async throws -> URL = { try await WebAuth.run($0, callbackScheme: $1) }
    ) {
        self.server = server
        self.registry = registry
        self.makeCore = makeCore
        self.browse = browse
    }

    /// Whether `link` looks like a family invitation (the core checks it exactly).
    static func isInvite(_ link: String) -> Bool {
        let t = link.trimmingCharacters(in: .whitespacesAndNewlines)
        return t.hasPrefix("reins-family://") || t.count == 43
    }

    func run(link: String) async {
        guard step != .working else { return }
        guard Self.isInvite(link) else {
            error = "This isn't a family invitation."
            return
        }
        step = .working
        error = nil
        var reserved: AccountProfile?
        do {
            let profile = try registry.beginManaged(serverUrl: server)
            reserved = profile
            let core = try makeCore(profile)
            let start = try await core.ssoBegin(serverUrl: server)
            guard let url = URL(string: start.url) else { throw ForeignError.Failed(reason: "The sign-in page didn't open.") }
            let callback = try await browse(url, start.callbackScheme)
            let outcome = try await core.ssoFinish(serverUrl: server, callbackUrl: callback.absoluteString, state: start.state, verifier: start.verifier)
            let family = try await core.familyJoin(link: link.trimmingCharacters(in: .whitespacesAndNewlines))
            let added = try registry.confirmFamily(id: profile.id, orgId: family.orgId, orgName: family.name, email: outcome.session.email)
            reserved = nil
            if let tag = try? await core.pushTag() { WorkProfiles.rememberPushTag(tag, profileId: added.id) }
            try? await core.registerDevice(fcmToken: nil)
            step = .added(label: added.label, id: added.id)
        } catch {
            if let reserved { try? registry.remove(id: reserved.id) }
            step = .link
            self.error = error.userMessage
        }
    }
}

struct AddFamilyAccountScreen: View {
    @State var flow: AddFamilyAccount
    @State private var link = ""

    var body: some View {
        QuietPage("Add family account") {
            VStack(alignment: .leading, spacing: 12) {
                if let error = flow.error {
                    QuietBanner(text: error).accessibilityIdentifier("addFamilyError")
                }
                switch flow.step {
                case .link, .working:
                    RaisedFields {
                        GlyphInputRow(glyph: .link) {
                            TextField("Invitation link", text: $link)
                                .textInputAutocapitalization(.never)
                                .keyboardType(.URL)
                                .autocorrectionDisabled()
                                .accessibilityIdentifier("familyInvite")
                        }
                    }
                    Button("Join with a new account") { Task { await flow.run(link: link) } }
                        .buttonStyle(SignalButtonStyle(.violet))
                        .disabled(flow.step == .working || link.isEmpty)
                        .accessibilityIdentifier("addFamilyContinue")
                case let .added(label, id):
                    Pill(label, tone: .neutral, symbol: FamilySymbol.name).accessibilityIdentifier("addedFamily")
                    Button { ProfileSwitch.switchTo(id, registry: flow.registry) } label: {
                        Label { Text("Switch to \(label)") } icon: { Image(systemName: FamilySymbol.name) }
                    }
                    .buttonStyle(SignalButtonStyle(.violet))
                    .accessibilityIdentifier("switchToFamily")
                }
            }
            .padding(16)
        }
    }
}

/// What a family's page asks of its core (the real `ReinsCore`; a fake in tests).
protocol FamilyCore: Sendable {
    func familyView() async throws -> FamilyView?
    func familyRequestLeave() async throws -> FamilyLeaveInfo
    func familyCancelLeave() async throws
    func familyConsentLeave(member: String) async throws
    func familyAnswerMinor(accept: Bool) async throws
}

/// The app's core, as `FamilyCore`.
struct CoreFamily: FamilyCore, @unchecked Sendable {
    let core: any ReinsCoreProtocol

    func familyView() async throws -> FamilyView? { try await core.familyView() }
    func familyRequestLeave() async throws -> FamilyLeaveInfo { try await core.familyRequestLeave() }
    func familyCancelLeave() async throws { try await core.familyCancelLeave() }
    func familyConsentLeave(member: String) async throws { try await core.familyConsentLeave(member: member) }
    func familyAnswerMinor(accept: Bool) async throws { try await core.familyAnswerMinor(accept: accept) }
}

/// A family profile's own page: what the family sees, leaving (after the notice, or a guardian's Allow for a minor),
/// a guardian's question, and, for guardians, the members' requests to leave.
@MainActor
@Observable
final class FamilyModel {
    private(set) var view: FamilyView?
    private(set) var error: String?
    private(set) var busy = false
    private let core: FamilyCore

    init(core: FamilyCore) { self.core = core }

    func load() async { await act {} }
    func leave() async { await act { _ = try await self.core.familyRequestLeave() } }
    func stay() async { await act { try await self.core.familyCancelLeave() } }
    func allow(_ member: String) async { await act { try await self.core.familyConsentLeave(member: member) } }
    func answerMinor(_ accept: Bool) async { await act { try await self.core.familyAnswerMinor(accept: accept) } }

    private func act(_ step: @escaping () async throws -> Void) async {
        busy = true
        error = nil
        defer { busy = false }
        do {
            try await step()
            view = try await core.familyView()
        } catch {
            self.error = error.userMessage
        }
    }

    /// "Leaves in 3 days": whole days left, rounded up.
    static func leavesIn(_ effectiveAt: Int64, now: Int64 = Int64(Date().timeIntervalSince1970)) -> String {
        let days = max(0, (effectiveAt - now + 86_399) / 86_400)
        switch days {
        case 0: return "Leaving now"
        case 1: return "Leaves in 1 day"
        default: return "Leaves in \(days) days"
        }
    }
}

struct FamilyScreen: View {
    @Environment(AppModel.self) private var model
    @State private var vm: FamilyModel?

    var body: some View {
        QuietPage(vm?.view.map { untrustedName($0.name, limit: 40) } ?? "Family") {
            if let vm {
                VStack(alignment: .leading, spacing: 12) {
                    if let error = vm.error { QuietBanner(text: error).accessibilityIdentifier("familyError") }
                    if let v = vm.view { content(v, vm) }
                }
                .padding(16)
            }
        }
        .task {
            let m = vm ?? FamilyModel(core: CoreFamily(core: model.core))
            vm = m
            await m.load()
        }
        .refreshable { await vm?.load() }
    }

    @ViewBuilder
    private func content(_ v: FamilyView, _ vm: FamilyModel) -> some View {
        Pill(untrustedName(v.name, limit: 40), tone: .neutral, symbol: FamilySymbol.name).accessibilityIdentifier("profileBadge")
        if v.minorProposed {
            QuietBanner(text: "Mark this account a minor's?", kind: .info).accessibilityIdentifier("minorAsk")
            HStack(spacing: 8) {
                Button("Accept") { Task { await vm.answerMinor(true) } }
                    .buttonStyle(SignalButtonStyle(.inverse)).disabled(vm.busy).accessibilityIdentifier("minorAccept")
                Button("Decline") { Task { await vm.answerMinor(false) } }
                    .buttonStyle(SignalButtonStyle(.outline)).disabled(vm.busy).accessibilityIdentifier("minorDecline")
            }
        }
        SectionHeader("What your family sees", glyph: .eye)
        RaisedGroup {
            ForEach(Array(v.sees.enumerated()), id: \.offset) { i, line in
                TileRow(line, glyph: .eye, chevron: false).accessibilityIdentifier("sees:\(i)")
            }
        }
        if v.guardian {
            ForEach(v.leaves, id: \.member) { ask in
                let who = untrustedName(ask.email, limit: 64)
                if let at = ask.effectiveAt {
                    QuietBanner(text: "\(who) · \(FamilyModel.leavesIn(at))", kind: .info).accessibilityIdentifier("leaving:\(ask.member)")
                } else {
                    QuietBanner(text: "\(who) asked to leave. Allow?", kind: .info).accessibilityIdentifier("leaveAsk:\(ask.member)")
                    Button("Allow") { Task { await vm.allow(ask.member) } }
                        .buttonStyle(SignalButtonStyle(.inverse)).disabled(vm.busy).accessibilityIdentifier("allowLeave:\(ask.member)")
                }
            }
        } else if let leave = v.leave {
            if leave.needsConsent {
                QuietBanner(text: "Waiting for a guardian", kind: .info).accessibilityIdentifier("leaveWaiting")
            } else {
                QuietBanner(text: FamilyModel.leavesIn(leave.effectiveAt ?? 0), kind: .info).accessibilityIdentifier("leaveCountdown")
            }
            Button("Stay") { Task { await vm.stay() } }
                .buttonStyle(SignalButtonStyle(.outline)).disabled(vm.busy).accessibilityIdentifier("familyStay")
        } else {
            Button(v.minor ? "Leave" : "Leave · \(v.noticeDays) days' notice") { Task { await vm.leave() } }
                .buttonStyle(SignalButtonStyle(.outline)).disabled(vm.busy).accessibilityIdentifier("familyLeave")
        }
    }
}
