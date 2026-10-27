import SwiftUI

/// The work profile's badge on cards and rows (06 §2): the org's name with a briefcase. Nothing in the Personal
/// profile. `onField`: on an ask's coloured field.
struct ProfileBadge: View {
    var onField = false
    var active: WorkProfiles.Active = WorkProfiles.active

    var body: some View {
        if let label = WorkProfiles.badge(active) {
            Pill(label, tone: onField ? .onFieldWash : .neutral, glyph: .briefcase)
                .accessibilityIdentifier("profileBadge")
        }
    }
}

/// Settings → Profiles: Personal and each work profile, the active one marked; another one is opened by switching.
/// Shown only where the server has organizations, or the phone already has a work profile.
struct ProfilesSection: View {
    let model: AppModel
    let server: String
    @State private var profiles: [AccountProfile] = []
    @State private var offered = false
    @State private var adding = false
    /// A work profile's org reviews waiting (nil: not known, or the Personal profile).
    @State private var reviews: Int?

    var body: some View {
        Group {
            if offered || profiles.contains(where: { $0.kind == .managed }) {
                SectionHeader("Profiles", glyph: .person)
                RaisedGroup {
                    if WorkProfiles.active.managed {
                        Button { model.show(.orgReviews) } label: {
                            TileRow("Reviews", glyph: .shieldCheck, value: reviews.flatMap { $0 > 0 ? String($0) : nil })
                        }
                        .buttonStyle(.plain)
                        .accessibilityIdentifier("orgReviewsRow")
                    }
                    ForEach(profiles, id: \.id) { p in
                        let current = p.id == WorkProfiles.active.id
                        Button {
                            if !current { ProfileSwitch.switchTo(p.id) }
                        } label: {
                            TileRow(WorkProfiles.label(p), glyph: p.kind == .managed ? .briefcase : .person,
                                    value: current ? "In use" : untrustedName(p.email ?? "", limit: 64))
                        }
                        .buttonStyle(.plain)
                        .accessibilityIdentifier("profile:\(p.id)")
                    }
                    Button { adding = true } label: {
                        TileRow("Add work account", glyph: .briefcase)
                    }
                    .buttonStyle(.plain)
                    .accessibilityIdentifier("addWorkAccount")
                }
            }
        }
        .task {
            profiles = WorkProfiles.registry().list()
            offered = (try? await model.core.orgsOffered(serverUrl: server)) ?? false
            if WorkProfiles.active.managed {
                // Reviews (O2) and web passkeys to confirm (O3).
                let passkeys = (try? await model.core.orgPasskeys().filter { !$0.confirmed }.count) ?? 0
                reviews = (try? await model.core.orgReviews().count).map { $0 + passkeys }
            }
        }
        .sheet(isPresented: $adding) {
            NavigationStack {
                AddWorkAccountScreen(flow: AddWorkAccount(server: server, makeCore: { try CoreFactory.makeWork($0) }))
            }
        }
    }
}

/// Switching profiles opens the other profile's core: the app moves to it in place, never by quitting.
enum ProfileSwitch {
    /// What to open once switched (a notification of that profile's: its reviews); taken by the relaunch.
    nonisolated(unsafe) static var then: DeepLink?

    /// Rebuilds the app on the newly active profile (`AppHost.switchProfile`). Tests replace it.
    nonisolated(unsafe) static var relaunch: @MainActor () -> Void = {
        Task { @MainActor in
            await AppHost.current?.switchProfile()
            guard let link = ProfileSwitch.then else { return }
            ProfileSwitch.then = nil
            guard let model = await AppHost.current?.ready() else { return }
            for _ in 0..<100 {
                guard case .loading = model.session else { break }
                try? await Task.sleep(for: .milliseconds(100))
            }
            await model.handle(link)
        }
    }

    @MainActor static func switchTo(_ id: String, registry: ProfileRegistryProtocol = WorkProfiles.registry()) {
        guard (try? registry.setActive(id: id)) != nil else { return }
        relaunch()
    }
}

/// "Add work account": a domain, then the org's sign-in, then this phone enrolled.
struct AddWorkAccountScreen: View {
    @State var flow: AddWorkAccount
    @State private var domain = ""

    var body: some View {
        QuietPage("Add work account") {
            VStack(alignment: .leading, spacing: 12) {
                if let error = flow.error {
                    QuietBanner(text: error).accessibilityIdentifier("addWorkError")
                }
                switch flow.step {
                case .domain, .working:
                    RaisedFields {
                        GlyphInputRow(glyph: .briefcase) {
                            TextField("Work domain, like acme.com", text: $domain)
                                .textInputAutocapitalization(.never)
                                .keyboardType(.URL)
                                .autocorrectionDisabled()
                                .accessibilityIdentifier("workDomain")
                        }
                    }
                    Button("Continue") { Task { await flow.run(domain: domain) } }
                        .buttonStyle(SignalButtonStyle(.violet))
                        .disabled(flow.step == .working || domain.isEmpty)
                        .accessibilityIdentifier("addWorkContinue")
                case let .added(label, id, enrolled):
                    // The org's name is shown with the briefcase only, never on its own.
                    Pill(label, tone: .neutral, glyph: .briefcase).accessibilityIdentifier("addedProfile")
                    QuietBanner(text: enrolled ? "Added. This phone is enrolled." : "Added.", kind: .info)
                    Button { ProfileSwitch.switchTo(id, registry: flow.registry) } label: {
                        Label { Text("Switch to \(label)") } icon: { GlyphIcon(.briefcase, size: 16) }
                    }
                        .buttonStyle(SignalButtonStyle(.violet))
                        .accessibilityIdentifier("switchToWork")
                }
            }
            .padding(16)
        }
    }
}
