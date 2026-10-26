#if DEBUG
import SwiftUI

/// The gallery's System page: the kit as the System board lays it out (design/next/iphone/previews/Next-System.png),
/// to compare side by side. `-signalGallery system/<section>` scrolls to a section: templates, icons, modes, outcomes,
/// core.
struct SystemGalleryPage: View {
    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            Group {
                caption("Page templates").id("templates")
                templates
                caption("Icon vocabulary").id("icons")
                IconVocabulary()
                caption("Modes · one component").id("modes")
                modes
                caption("Outcomes · activity rows").id("outcomes")
                outcomes
                caption("Core components").id("core")
            }
            .padding(.horizontal, 24)
            CoreComponents()
        }
        .padding(.bottom, 40)
    }

    private func caption(_ text: String) -> some View {
        Text(text.uppercased())
            .font(RFont.mono(11.5))
            .tracking(11.5 * 0.16)
            .foregroundStyle(Palette.inkMeta)
            .padding(.top, 30)
            .padding(.bottom, 12)
    }

    // MARK: Templates

    private var templates: some View {
        VStack(alignment: .leading, spacing: 20) {
            // Hero: Activity, the ask, running modes.
            VStack(alignment: .leading, spacing: 12) {
                HStack(spacing: 8) {
                    Text("Activity").textRole(.title).frame(maxWidth: .infinity, alignment: .leading)
                    ModeChip(mode: .manual, chevron: true)
                    RoundHeaderButton(glyph: .search, label: "Search", size: 36) {}
                }
                WaitCardShell(kind: .push, label: "GitHub · Push", ask: AskText("Push 8 commits to", target: "octo/app", role: .cardAsk)) {
                    RequestIcon(spec: AskIcon(mark: .service("github"), action: .gitBranch, kind: .push), size: 30, ring: FieldKind.push.fill)
                } timer: {
                    HStack(spacing: 10) {
                        ProviderAvatar(provider: Providers.byKey("claude")!, size: 26)
                        RingClock(text: "9:40", fraction: 0.8)
                    }
                } actions: {
                    VStack(alignment: .leading, spacing: 12) {
                        Text("feature/passkeys").font(RFont.mono(13)).foregroundStyle(.white.opacity(0.85)).padding(.top, -4)
                        WaitCardButtons(pick: .allow(97), onPick: {}, onReview: {})
                    }
                }
            }
            // Standard: Trust, Connections, Vault.
            VStack(alignment: .leading, spacing: 0) {
                HStack {
                    Text("Trust").textRole(.title).frame(maxWidth: .infinity, alignment: .leading)
                    RoundHeaderButton(glyph: .plus, label: "Add", size: 36) {}
                }
                .padding(.bottom, 12)
                ModeCard(mode: .manual, actionTitle: "Change") {}.padding(.horizontal, -16)
                SectionHeader("Grants", glyph: .key, trailing: "6").padding(.horizontal, -24)
                RaisedGroup {
                    TileRow("Read emails from @bank.com", value: "50m") {
                        ServiceAvatar(service: "gmail", size: 32)
                    } trailing: { EmptyView() }
                }
                .padding(.horizontal, -16)
            }
            // Quiet: Settings and below.
            VStack(alignment: .leading, spacing: 0) {
                RoundHeaderButton(glyph: .chevronLeft, label: "Back", size: 36) {}
                Text("Sounds").textRole(.quietTitle).padding(.top, 8)
                SectionHeader("Sounds", quiet: true).padding(.horizontal, -20)
                FlatGroup {
                    QuietSwitchRow(title: "Chimes", glyph: .bell, on: true)
                    QuietSwitchRow(title: "Requests", glyph: .tray, on: true)
                    QuietSwitchRow(title: "Alerts", glyph: .warning, on: false)
                }
                .padding(.horizontal, -20)
            }
        }
        .foregroundStyle(Palette.ink)
    }

    // MARK: Modes

    private var modes: some View {
        VStack(alignment: .leading, spacing: 12) {
            FlowRow(spacing: 8) {
                ModeChip(mode: .manual, chevron: true)
                ModeChip(mode: .auto, chevron: true)
                ModeChip(mode: .overnight(left: "5:59"), chevron: true)
                ModeChip(mode: .lockdown, chevron: true)
                ModeChip(mode: .bypass(left: "47:12"), chevron: true)
            }
            VStack(spacing: 14) {
                ModeCard(mode: .manual, actionTitle: "Change") {}
                ModeCard(mode: .auto, actionTitle: "Change") {}
                ModeCard(mode: .overnight, clock: ("5:59:12", 0.75), actionTitle: "Stop") {}
                ModeCard(mode: .lockdown, actionTitle: "End") {}
                ModeCard(mode: .bypass, clock: ("47:12", 0.4), actionTitle: "Stop") {}
            }
            .padding(.horizontal, -16)
        }
    }

    // MARK: Outcomes

    private var outcomes: some View {
        VStack(alignment: .leading, spacing: 0) {
            ForEach(Array(Self.tags.enumerated()), id: \.offset) { _, tag in
                HStack(spacing: 10) {
                    OutcomeMark(tag.0).frame(width: 76, alignment: .trailing)
                    Text(tag.0.spoken).font(RFont.sans(14.5)).foregroundStyle(Palette.ink2)
                }
                .frame(height: 48)
            }
            RaisedGroup {
                ActivityRowView(runs: [.init(text: "Claude Code pushed 3 commits to "), .init(text: "dkat/reins", target: true)], time: "1m", outcome: .autoApproved) {
                    RequestIcon(spec: AskIcon(mark: .service("github"), action: .gitBranch, kind: .push), size: 36, ring: Palette.paperRaised)
                }
                ActivityRowView(
                    runs: [.init(text: "notes-bot tried to email "), .init(text: "backup-svc", target: true)], time: "5m", outcome: .autoDenied,
                    risk: "Not in your contacts"
                ) {
                    RequestIcon(spec: AskIcon(mark: .service("gmail"), action: .send, kind: .push), size: 36, ring: Palette.paperRaised)
                }
                ActivityRowView(runs: [.init(text: "My ChatGPT read "), .init(text: "2 emails", target: true)], time: "10m", outcome: .bypass) {
                    RequestIcon(spec: AskIcon(mark: .service("gmail"), action: .eye, kind: .read), size: 36, ring: Palette.paperRaised)
                }
            }
            .padding(.horizontal, -16)
            .padding(.top, 8)
        }
    }

    private static let tags: [(Outcome, Void)] = [
        (.approved, ()), (.denied, ()), (.autoApproved, ()), (.autoDenied, ()), (.grant, ()), (.bypass, ()), (.locked, ()), (.night, ()), (.failed, ()),
    ]
}

/// The icon vocabulary: one glyph, one meaning, each on a sunken tile with its word.
private struct IconVocabulary: View {
    private static let groups: [(String, [(Glyph?, ModeKind?, String)])] = [
        ("Modes", [(nil, .manual, "Manual"), (nil, .auto, "Autopilot"), (nil, .overnight, "Overnight"), (nil, .lockdown, "Lockdown"), (nil, .bypass, "Bypass")]),
        ("Outcomes", [(.check, nil, "Approved"), (.close, nil, "Denied"), (.key, nil, "Grant"), (.circleAlert, nil, "Failed"), (.undo, nil, "Wrong / undo"), (.warning, nil, "Risk")]),
        ("Actions", [
            (.gitBranch, nil, "Push"), (.send, nil, "Send"), (.eye, nil, "Read"), (.trash, nil, "Delete"), (.pencil, nil, "Edit"), (.plus, nil, "Create"),
            (.play, nil, "Run"), (.search, nil, "Search"), (.vault, nil, "Secret"), (.link, nil, "Pair"), (.phone, nil, "New phone"), (.file, nil, "File"),
        ]),
        ("Facts", [
            (.timer, nil, "Time"), (.person, nil, "Account"), (.gitBranch, nil, "Branch"), (.gitCommit, nil, "Commit"), (.globe, nil, "Host"),
            (.mail, nil, "Email"), (.file, nil, "File"), (.server, nil, "Server"), (.calendar, nil, "Date"), (.token, nil, "Uses"),
            (.timer, nil, "Duration"), (.terminal, nil, "Command"), (.phone, nil, "Phone"), (.laptop, nil, "Computer"),
        ]),
        ("States", [(.lock, nil, "Always asks"), (.help, nil, "Help"), (.more, nil, "More"), (.settings, nil, "Settings")]),
    ]

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            ForEach(Self.groups, id: \.0) { group in
                Text(group.0).font(RFont.sans(14, .semibold)).foregroundStyle(Palette.ink).padding(.top, 8)
                LazyVGrid(columns: Array(repeating: GridItem(.flexible(), spacing: 4), count: 5), spacing: 10) {
                    ForEach(Array(group.1.enumerated()), id: \.offset) { _, item in
                        VStack(spacing: 6) {
                            RoundedRectangle(cornerRadius: 10, style: .continuous)
                                .fill(Palette.paperSunken)
                                .frame(width: 40, height: 40)
                                .overlay {
                                    if let mode = item.1 {
                                        ModeGlyph(mode, size: 18).foregroundStyle(mode == .manual ? Palette.ink : mode.ink)
                                    } else if let glyph = item.0 {
                                        GlyphIcon(glyph, size: 18).foregroundStyle(Palette.ink2)
                                    }
                                }
                            Text(item.2).font(RFont.sans(12)).foregroundStyle(Palette.ink2).multilineTextAlignment(.center).lineLimit(2)
                        }
                    }
                }
            }
        }
    }
}

/// The core components column: buttons, choosing, controls, chips, fields, groups and quiet rows.
private struct CoreComponents: View {
    @State private var on = true
    @State private var picked = 0
    @State private var chosen = 0
    @State private var stepper = 5

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            head("Buttons")
            VStack(spacing: 12) {
                HStack(spacing: 10) {
                    Button("Deny") {}.buttonStyle(SignalButtonStyle(.outline))
                    Button {} label: { Label("Approve", glyph: .faceId) }.buttonStyle(SignalButtonStyle(.inverse))
                }
                HStack(spacing: 10) {
                    Button("Review") {}.buttonStyle(SignalButtonStyle(.soft))
                    Button {} label: { Label("Hold to delete", glyph: .trash) }.buttonStyle(SignalButtonStyle(.danger))
                }
            }
            .padding(.horizontal, 24)
            head("Choosing")
            VStack(spacing: 12) {
                Chooser([
                    .init(id: "once", title: "Once", selected: picked == 0) { picked = 0 },
                    .init(id: "hour", title: "1 hour", selected: picked == 1) { picked = 1 },
                    .init(id: "day", title: "1 day", selected: picked == 2) { picked = 2 },
                    .init(id: "custom", title: "Custom", selected: picked == 3, more: true) { picked = 3 },
                ])
                Chooser.asked("Every time")
                HStack(spacing: 12) {
                    Toggle("", isOn: $on).toggleStyle(SignalSwitchStyle()).labelsHidden().fixedSize()
                    SignalSwitchTrack(on: false)
                    SignalCheckbox(on: true)
                    RadioDot(selected: true)
                    SignalStepper(value: "\(stepper)", onDecrease: { stepper -= 1 }, onIncrease: { stepper += 1 })
                }
                HStack(spacing: 8) {
                    SelectChip(title: "Claude", selected: chosen == 0) { ProviderAvatar(provider: Providers.byKey("claude")!, size: 22) } action: { chosen = 0 }
                    SelectChip(title: "Cursor", selected: chosen == 1) { ProviderAvatar(provider: Providers.byKey("cursor")!, size: 22) } action: { chosen = 1 }
                    SelectChip(title: "M2A", selected: chosen == 2) { GlyphIcon(.laptop, size: 18) } action: { chosen = 2 }
                }
            }
            .padding(.horizontal, 24)
            head("Groups")
            SectionHeader("Standard · raised group", glyph: .link, trailing: "2")
            RaisedGroup {
                TileRow("Claude") { ProviderAvatar(provider: Providers.byKey("claude")!, size: 32) } trailing: { EmptyView() }
                TileRow("Cursor") { ProviderAvatar(provider: Providers.byKey("cursor")!, size: 32) } trailing: { EmptyView() }
            }
            SectionHeader("Quiet · flat rows", quiet: true)
            FlatGroup {
                TileRow("Devices", glyph: .devices, value: "2").padding(.horizontal, -14)
                TileRow("Appearance", glyph: .appearance, value: "System").padding(.horizontal, -14)
            }
        }
    }

    private func head(_ text: String) -> some View {
        Text(text).font(RFont.sans(14, .semibold)).foregroundStyle(Palette.ink).padding(.top, 24).padding(.bottom, 12).padding(.horizontal, 24)
    }
}

private extension Label where Title == Text, Icon == GlyphIcon {
    init(_ title: String, glyph: Glyph) {
        self.init { Text(title) } icon: { GlyphIcon(glyph, size: 18) }
    }
}

/// A quiet row with a switch (Sounds).
private struct QuietSwitchRow: View {
    var title: String
    var glyph: Glyph
    @State var on: Bool

    var body: some View {
        HStack(spacing: 12) {
            GlyphTile(glyph)
            Toggle(title, isOn: $on).toggleStyle(SignalSwitchStyle()).font(TextRole.row.font())
        }
        .frame(minHeight: 56)
    }
}
#endif
