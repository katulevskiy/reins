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

    var body: some View {
        Group {
            if offered || profiles.contains(where: { $0.kind == .managed }) {
                SectionHeader("Profiles", glyph: .person)
                RaisedGroup {
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
        }
        .sheet(isPresented: $adding) {
            NavigationStack {
                AddWorkAccountScreen(flow: AddWorkAccount(server: server, makeCore: { try CoreFactory.makeWork($0) }))
            }
        }
    }
}

/// Switching profiles opens the other profile's core: the app starts again on it.
enum ProfileSwitch {
    /// Tests replace the relaunch.
    nonisolated(unsafe) static var relaunch: () -> Void = { exit(0) }

    static func switchTo(_ id: String, registry: ProfileRegistryProtocol = WorkProfiles.registry()) {
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
                    QuietBanner(text: enrolled ? "\(label) added. This phone is enrolled." : "\(label) added.", kind: .info)
                    Button("Switch to \(label)") { ProfileSwitch.switchTo(id, registry: flow.registry) }
                        .buttonStyle(SignalButtonStyle(.violet))
                        .accessibilityIdentifier("switchToWork")
                }
            }
            .padding(16)
        }
    }
}
