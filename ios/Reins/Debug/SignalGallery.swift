#if DEBUG
import SwiftUI

/// The design gallery (debug builds; Settings > Developer, or `-signalGallery [page]` at launch): every component, field,
/// type role and glyph of the foundation, in the system's light or dark, to compare with design/next/iphone/previews.
struct SignalGallery: View {
    enum Page: String, CaseIterable, Identifiable {
        case system, type, colour, buttons, controls, cards, field, glyphs

        var id: String { rawValue }
        var title: String { rawValue.capitalized }
    }

    static var requested: Bool { ProcessInfo.processInfo.arguments.contains("-signalGallery") }

    /// `-signalGallery buttons/hold`: the page, and optionally the section label to scroll to.
    private static var launch: (page: Page, anchor: String?) {
        let args = ProcessInfo.processInfo.arguments
        guard let i = args.firstIndex(of: "-signalGallery"), i + 1 < args.count else { return (.system, nil) }
        let parts = args[i + 1].split(separator: "/", maxSplits: 1).map(String.init)
        return (Page(rawValue: parts[0]) ?? .system, parts.count > 1 ? parts[1] : nil)
    }

    static var requestedPage: Page { launch.page }

    @State var page: Page
    var onClose: (() -> Void)?

    var body: some View {
        VStack(spacing: 0) {
            if page == .field { FieldPage() }
            ScrollView(.horizontal, showsIndicators: false) {
                HStack(spacing: 8) {
                    if let onClose { RoundHeaderButton(glyph: .close, label: "Close", size: 34, action: onClose) }
                    ForEach(Page.allCases) { p in
                        FilterChip(title: p.title, selected: p == page) { page = p }
                            .accessibilityIdentifier("gallery:\(p.rawValue)")
                    }
                }
                .padding(.horizontal, 16)
                .padding(.vertical, 8)
            }
            ScrollViewReader { reader in
                ScrollView {
                    Group {
                        switch page {
                        case .system: SystemGalleryPage().padding(.horizontal, -24)
                        case .type: TypePage()
                        case .colour: ColourPage()
                        case .buttons: ButtonsPage()
                        case .controls: ControlsPage()
                        case .cards: CardsPage()
                        case .field: FieldNotes()
                        case .glyphs: GlyphsPage()
                        }
                    }
                    .padding(.horizontal, 24)
                    .padding(.bottom, 40)
                    .frame(maxWidth: .infinity, alignment: .leading)
                }
                .task {
                    guard let anchor = Self.launch.anchor, Self.launch.page == page else { return }
                    try? await Task.sleep(for: .milliseconds(900))
                    reader.scrollTo(anchor, anchor: .top)
                }
            }
        }
        .background(Palette.paper.ignoresSafeArea())
        .signalMenuHost()
    }
}

/// A section label above a group, `space-26` above it.
private struct Label: View {
    var text: String
    init(_ text: String) { self.text = text }

    var body: some View {
        Text(text).textRole(.section).foregroundStyle(Palette.inkMeta).padding(.top, 26).padding(.bottom, 10)
            .id(text.split(separator: " ").first.map { $0.lowercased() } ?? text)
    }
}

// MARK: Type

private struct TypePage: View {
    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            Text("Type").textRole(.title).foregroundStyle(Palette.ink).padding(.top, 8)
            Label("Display · Sora 500")
            role(.ask) { AskText("Push 8 commits to", target: "octo/app").foregroundStyle(Palette.ink) }
            role(.title) { Text("Activity").textRole(.title) }
            role(.cardAsk) { AskText("Push 8 commits to", target: "octo/app", role: .cardAsk) }
            role(.quietTitle) { Text("Sounds").textRole(.quietTitle) }
            Label("Cyrillic · Geologica under Sora")
            AskText("Отправить 8 коммитов в", target: "octo/app", role: .cardAsk).padding(.vertical, 10)
            Label("Numbers · tabular in timers")
            HStack(spacing: 28) {
                Text("9:40").foregroundStyle(Palette.ink)
                Text("97%").foregroundStyle(Palette.success)
                Text("47:12").foregroundStyle(Palette.danger)
            }
            .font(RFont.display(TextRole.title.size)).tabular().padding(.vertical, 10)
            role(.clock) { Text("47:12").textRole(.clock) }
            Label("Text · Sora 400 / 500 / 600")
            role(.body) {
                Text("Claude Code pushed 3 commits to \(Text("dkat/reins").font(TextRole.body.font(target: true)))").textRole(.body)
            }
            role(.row) { Text("Notification channels").textRole(.row) }
            role(.button) { Text("Approve").textRole(.button) }
            role(.section) { Text("Today").textRole(.section).foregroundStyle(Palette.ink2) }
            role(.meta) { Text("13m · Oct 23 · 14:02").textRole(.meta).foregroundStyle(Palette.inkMeta) }
            role(.tag) { Pill("Auto", tone: .violet) }
            Label("Machine values · JetBrains Mono")
            role(.value) { Text("feature/passkeys · 9f3c2a1").textRole(.value) }
            role(.detail) { Text("backup-svc@protonmail.example").textRole(.detail).foregroundStyle(Palette.ink2) }
            role(.codes) { Text("4821 9930").textRole(.codes) }
            role(.code) {
                Text("if (await browserSupportsPasskeys()) {\n  return post(\"/login/passkey\", cred);\n}").textRole(.code)
                    .padding(12).frame(maxWidth: .infinity, alignment: .leading)
                    .background(Palette.paperSunken, in: RoundedRectangle(cornerRadius: 14, style: .continuous))
            }
            Label("Asks reflow before they shrink")
            AskText("Delete the repository", target: "octo/an-unusually-long-repository-name-for-testing")
                .foregroundStyle(Palette.ink)
        }
        .foregroundStyle(Palette.ink)
    }

    private func role<Content: View>(_ role: TextRole, @ViewBuilder _ content: () -> Content) -> some View {
        VStack(alignment: .leading, spacing: 4) {
            Text(String(describing: role)).textRole(.section).foregroundStyle(Palette.inkFaint)
            content()
        }
        .padding(.vertical, 10)
        .frame(maxWidth: .infinity, alignment: .leading)
        .overlay(alignment: .bottom) { Rectangle().fill(Palette.rule).frame(height: 1) }
    }
}

// MARK: Colour

private struct ColourPage: View {
    @AppStorage(Appearance.grainKey, store: AppGroup.defaults) private var grain = true

    private static let words: [FieldKind: (String, String)] = [
        .push: ("Send", "it"), .read: ("Look", "inside"), .delete: ("For", "good"), .grant: ("For a", "while"),
        .pair: ("Say", "hello"), .accounts: ("Keys", "change"), .warn: ("Fix", "me"),
    ]

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            Text("Colour").textRole(.title).foregroundStyle(Palette.ink).padding(.top, 8)
            Toggle(isOn: $grain) {
                VStack(alignment: .leading, spacing: 1) {
                    Text("Grain on fields").textRole(.row).foregroundStyle(Palette.ink)
                    Text("A painted finish on requests").textRole(.meta).foregroundStyle(Palette.inkMeta)
                }
            }
            .padding(.top, 12)
            Label("Fields · the same in light and dark")
            LazyVGrid(columns: [GridItem(.adaptive(minimum: 98), spacing: 10)], spacing: 10) {
                ForEach(FieldKind.allCases, id: \.self) { kind in
                    let words = Self.words[kind] ?? ("", "")
                    VStack(alignment: .leading) {
                        Text(kind.rawValue).textRole(.section).foregroundStyle(Palette.onField2)
                        Spacer(minLength: 18)
                        AskText(words.0, target: words.1, role: .cardAsk).foregroundStyle(Palette.onField)
                    }
                    .padding(12)
                    .frame(maxWidth: .infinity, minHeight: 110, alignment: .leading)
                    .fieldBackground(kind, in: RoundedRectangle(cornerRadius: 18, style: .continuous))
                }
            }
            Label("Paper and ink")
            ForEach(Self.swatches, id: \.0) { name, color in
                HStack(spacing: 14) {
                    RoundedRectangle(cornerRadius: 9, style: .continuous)
                        .fill(color)
                        .overlay(RoundedRectangle(cornerRadius: 9, style: .continuous).strokeBorder(Palette.rule, lineWidth: 1))
                        .frame(width: 34, height: 34)
                    Text(name).textRole(.row).foregroundStyle(Palette.ink)
                    Spacer()
                }
                .padding(.vertical, 6)
                .overlay(alignment: .bottom) { Rectangle().fill(Palette.rule).frame(height: 1) }
            }
            Label("Kind tiles · the ink at 13 %")
            HStack(spacing: 10) {
                ForEach(FieldKind.allCases, id: \.self) { KindTile($0) }
            }
            Label("Flat · pigment")
            HStack(spacing: 10) {
                Text("Flat").textRole(.section).foregroundStyle(Palette.onField2)
                    .padding(14).frame(maxWidth: .infinity, minHeight: 90, alignment: .topLeading)
                    .background(Palette.fieldPush, in: RoundedRectangle(cornerRadius: 18, style: .continuous))
                Text("Pigment").textRole(.section).foregroundStyle(Palette.onField2)
                    .padding(14).frame(maxWidth: .infinity, minHeight: 90, alignment: .topLeading)
                    .fieldBackground(.push, in: RoundedRectangle(cornerRadius: 18, style: .continuous))
            }
        }
    }

    private static let swatches: [(String, Color)] = [
        ("paper", Palette.paper), ("paper-raised", Palette.paperRaised), ("paper-sunken", Palette.paperSunken),
        ("rule", Palette.rule), ("rule-2", Palette.rule2), ("ink", Palette.ink), ("ink-2", Palette.ink2),
        ("ink-meta", Palette.inkMeta), ("ink-faint", Palette.inkFaint), ("inverse", Palette.inverse), ("violet", Palette.violet),
        ("violet-fill", Palette.violetFill), ("success", Palette.success), ("danger", Palette.danger),
        ("danger-wash", Palette.dangerWash), ("warning", Palette.warning),
    ]
}

// MARK: Buttons

private struct ButtonsPage: View {
    @State private var held = 0

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            Text("Buttons").textRole(.title).foregroundStyle(Palette.ink).padding(.top, 8)
            Label("Decide")
            RatioRow(spacing: 10, ratio: 1 / 1.35) {
                Button("Deny") {}.buttonStyle(SignalButtonStyle(.outline))
                Button {} label: { SwiftUI.Label { Text("Approve") } icon: { GlyphIcon(.faceId) } }
                    .buttonStyle(SignalButtonStyle(.inverse))
            }
            .frame(height: 54)
            RatioRow(spacing: 10, ratio: 1 / 1.35) {
                Button("Deny") {}.buttonStyle(SignalButtonStyle(.outline))
                Button {} label: { SwiftUI.Label { Text("Send") } icon: { GlyphIcon(.fingerprint) } }
                    .buttonStyle(SignalButtonStyle(.inverse))
            }
            .frame(height: 54)
            Label("Small")
            FlowRow {
                Button("This was wrong") {}.buttonStyle(SignalButtonStyle(.soft, size: .small, fill: false))
                Button("End now") {}.buttonStyle(SignalButtonStyle(.danger, size: .small, fill: false))
                Button("Allow") {}.buttonStyle(SignalButtonStyle(.violet, size: .small, fill: false))
                Text("Pressed").font(TextRole.buttonSmall.font()).foregroundStyle(Palette.onInverse)
                    .padding(.horizontal, 16).frame(height: 40).background(Palette.inverse, in: Capsule()).opacity(0.55)
                Button("Disabled") {}.buttonStyle(SignalButtonStyle(.inverse, size: .small, fill: false)).disabled(true)
            }
            Label("On a field")
            HStack(spacing: 8) {
                Button("Review") {}.buttonStyle(SignalButtonStyle(.onField, size: .small))
                Button("Deny") {}.buttonStyle(SignalButtonStyle(.onFieldWash, size: .small))
                RoundHeaderButton(glyph: .close, label: "Close", onField: true) {}
            }
            .padding(14)
            .fieldBackground(.push, in: RoundedRectangle(cornerRadius: 20, style: .continuous))
            Label("Focus · header buttons")
            HStack(spacing: 12) {
                Text("Focused").font(TextRole.buttonSmall.font()).foregroundStyle(Palette.onInverse)
                    .padding(.horizontal, 16).frame(height: 40).background(Palette.inverse, in: Capsule())
                    .signalFocusRing(true)
                RoundHeaderButton(glyph: .plus, label: "Add") {}
                RoundHeaderButton(glyph: .filter, label: "Filter") {}
                RoundHeaderButton(glyph: .chevronLeft, label: "Back") {}
            }
            Label("Hold · irreversible · 1.2 s")
            HoldButton(title: "Hold to delete", symbol: "trash", doneTitle: "Deleted") { held += 1 }
            HoldButton(title: "Hold to delete", symbol: "trash", still: 0.58) {}
            HoldButton(title: "Hold to delete", doneTitle: "Deleted", onConfirm: {})
                .modifier(DoneOnAppear())
            Text("Held \(held) times. Letting go drains it in 180 ms.").textRole(.meta).foregroundStyle(Palette.inkMeta)
            Label("Allow for · the chooser")
            ChooserDemo()
        }
    }
}

/// Shows a hold button in its done state, for the board.
private struct DoneOnAppear: ViewModifier {
    func body(content: Content) -> some View {
        content.overlay {
            HStack(spacing: 8) {
                GlyphIcon(.check)
                Text("Deleted")
            }
            .textRole(.button)
            .foregroundStyle(.white)
            .frame(maxWidth: .infinity, maxHeight: .infinity)
            .background(Palette.danger, in: Capsule())
            .allowsHitTesting(false)
        }
    }
}

private struct ChooserDemo: View {
    @State private var pick = "once"
    @State private var long = "week"

    var body: some View {
        VStack(spacing: 10) {
            Chooser(["once", "hour", "day"].map { id in
                Chooser.Segment(id: "c1:\(id)", title: ["once": "Once", "hour": "1 hour", "day": "1 day"][id]!, selected: pick == id) {
                    pick = id
                }
            } + [Chooser.Segment(id: "c1:custom", title: "Custom", selected: false, more: true, menu: [
                Chooser.MenuItem(id: "c1:week", title: "1 week") {}, Chooser.MenuItem(id: "c1:month", title: "30 days") {},
            ])])
            Chooser([
                Chooser.Segment(id: "c2:once", title: "Once", selected: long == "once") { long = "once" },
                Chooser.Segment(id: "c2:8h", title: "8 hours", selected: long == "8h") { long = "8h" },
                Chooser.Segment(id: "c2:day", title: "1 day", selected: long == "day") { long = "day" },
                Chooser.Segment(id: "c2:week", title: "1 week", selected: long == "week", more: true) { long = "week" },
            ])
            Chooser.asked()
            ChooserSummary(label: "Allow for", value: "1 hour · at most 5 uses") {}
        }
    }
}

// MARK: Controls

private struct ControlsPage: View {
    @State private var mode = "auto"
    @State private var on = true
    @State private var off = false
    @State private var count = 5
    @State private var filter = "Autopilot"

    private var items: [SignalMenuItem] {
        [
            SignalMenuItem(id: "manual", title: "Manual", subtitle: "You decide, with a hint", glyph: .person),
            SignalMenuItem(id: "auto", title: "Auto", subtitle: "Decides the easy ones", glyph: .autopilot, tone: .solid(Palette.violetFill)),
            SignalMenuItem(id: "lockdown", title: "Lockdown", subtitle: "Deny everything now", glyph: .lock, tone: .kind(.warn)),
            SignalMenuItem(
                id: "bypass", title: "Bypass…", subtitle: "Approve everything for a while", glyph: .autopilot, tone: .danger,
                separated: true, more: true
            ),
        ]
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            Text("Controls").textRole(.title).foregroundStyle(Palette.ink).padding(.top, 8)
            Label("Menus · grow from their trigger")
            HStack {
                Spacer()
                ModeChip(mode: mode == "manual" ? .manual : mode == "lockdown" ? .lockdown : .auto, chevron: true)
                    .signalMenu(id: "galleryMode", items: items, selected: mode) { mode = $0 == "bypass" ? mode : $0 }
                    .accessibilityIdentifier("galleryModeChip")
            }
            HStack {
                Spacer()
                SignalMenuPanel(items: items, selected: "auto").frame(width: 290)
            }
            Text("Tap the chip, or press it and slide down the rows: each row crossed ticks.")
                .textRole(.meta).foregroundStyle(Palette.inkMeta)
            Label("Mode chip · in every header")
            FlowRow {
                ModeChip(mode: .manual)
                ModeChip(mode: .auto)
                ModeChip(mode: .lockdown)
                ModeChip(mode: .bypass(left: "14:05"))
                ModeChip(mode: .overnight(left: "5:59"))
            }
            Label("Controls")
            HStack(spacing: 16) {
                Toggle("On", isOn: $on).labelsHidden().fixedSize()
                Toggle("Off", isOn: $off).labelsHidden().fixedSize()
                SignalCheckbox(on: true)
                SignalCheckbox(on: false)
                RadioMark(on: true)
                RadioMark(on: false)
            }
            HStack {
                SignalStepper(value: "\(count)", canDecrease: count > 1, onDecrease: { count -= 1 }, onIncrease: { count += 1 })
                Spacer()
            }
            FlowRow {
                ForEach(["All", "Autopilot", "Denied", "Grants"], id: \.self) { title in
                    FilterChip(title: title, selected: filter == title) { filter = title }
                }
            }
            Label("Pills · always a word")
            FlowRow {
                Pill("Auto", tone: .violet)
                Pill("New", tone: .success)
                Pill("Denied", tone: .danger)
                Pill("Hidden", tone: .warning)
                Pill("Code")
                Pill("Read only", tone: .kind(.read))
                OutcomePill(text: "Allowed", tone: .good)
            }
            Label("Timer")
            HStack(spacing: 16) {
                let now = Int64(Date().timeIntervalSince1970)
                TimerPill(createdAt: now - 60, waitUntil: now + 72)
                TimerPill(createdAt: now - 60, waitUntil: now + 72, onField: true)
                    .padding(10).fieldBackground(.push, in: Capsule())
                TimerPill(createdAt: now - 60, waitUntil: now + 12, onField: true)
                    .padding(10).fieldBackground(.read, in: Capsule())
            }
        }
    }
}

// MARK: Cards and rows

private struct CardsPage: View {
    @State private var tab = "activity"

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            Text("Cards").textRole(.title).foregroundStyle(Palette.ink).padding(.top, 8)
            Label("Waiting cards · Autopilot's pick first")
            WaitCardShell(kind: .push, label: "Gmail · Send", ask: AskText("Send an email to", target: "Anna Petrova", role: .cardAsk)) {
                ProviderMark(provider: "claude")
            } timer: {
                TimerPill(createdAt: Int64(Date().timeIntervalSince1970) - 30, waitUntil: Int64(Date().timeIntervalSince1970) + 72, onField: true)
            } actions: {
                WaitCardButtons(pick: .allow(97), onPick: {}, onReview: {})
            }
            WaitCardShell(kind: .read, label: "Gmail · Search", ask: AskText("Search mail for", target: "“invoice”", role: .cardAsk)) {
                ProviderMark(provider: "openai")
            } timer: {
                TimerPill(createdAt: Int64(Date().timeIntervalSince1970) - 60, waitUntil: Int64(Date().timeIntervalSince1970) + 12, onField: true)
            } actions: {
                WaitCardButtons(pick: .unsure, onPick: {}, onReview: {})
            }
            WaitCardShell(kind: .delete, label: "GitHub · Delete", ask: AskText("Delete the repo", target: "octo/old-api", role: .cardAsk)) {
                ProviderMark(provider: "claude")
            } timer: {
                Text("4:51").font(RFont.mono(13)).foregroundStyle(.white)
            } actions: {
                WaitCardButtons(pick: .alwaysAsks, onPick: {}, onReview: {})
            }
            Label("Rows · hairlines, never cards")
            VStack(spacing: 0) {
                HairlineFact(key: "Branch", value: "feature/passkeys", mono: true) { Pill("New", tone: .success) }
                HairlineFact(key: "8 commits", value: nil, chevron: true) {
                    Text("\(Text("+412").foregroundStyle(Palette.success)) \(Text("−96").foregroundStyle(Palette.danger))").font(RFont.mono(13))
                }
                HairlineRow(title: "Pushed 8 commits to octo/app", subtitle: "Claude Code · 4 min") {
                    KindTile(.push, glyph: .gitBranch)
                } trailing: {
                    Text("Auto").textRole(.tag).foregroundStyle(Palette.violet)
                }
                HairlineRow(title: "Read 2 emails from Stripe", subtitle: "ChatGPT · 18 min") {
                    KindTile(.read, glyph: .mail)
                } trailing: {
                    GlyphIcon(.check, size: 18).foregroundStyle(Palette.success)
                }
                HairlineRow(title: "Delete branch old/experiment", subtitle: "Cursor · 1 h") {
                    KindTile(.delete, glyph: .trash)
                } trailing: {
                    Text("Denied").textRole(.tag).foregroundStyle(Palette.danger)
                }
                SwitchRow()
            }
            Label("Tab bar")
            HStack {
                Spacer()
                SignalTabBar(items: [
                    .init(tab: "activity", title: "Activity", glyph: .activity, identifier: "gtab:activity"),
                    .init(tab: "trust", title: "Trust", glyph: .shield, identifier: "gtab:trust", dot: Palette.fieldDelete),
                    .init(tab: "connections", title: "Connections", glyph: .link, identifier: "gtab:connections"),
                    .init(tab: "settings", title: "Settings", glyph: .settings, identifier: "gtab:settings"),
                ], selection: $tab)
                Spacer()
            }
            .padding(.vertical, 8)
            Label("Calm · ink drawings for empty states")
            EmptyState.calm("All quiet on the water.")
            Illustration(.kite).frame(maxWidth: 260).frame(maxWidth: .infinity)
        }
    }
}

private struct SwitchRow: View {
    @State private var on = true

    var body: some View {
        Toggle(isOn: $on) {
            VStack(alignment: .leading, spacing: 1) {
                Text("Download on Wi-Fi only").textRole(.row).foregroundStyle(Palette.ink)
                Text("The model is 370 MB").textRole(.meta).foregroundStyle(Palette.inkMeta)
            }
        }
        .padding(.vertical, 9)
        .frame(minHeight: 56)
    }
}

/// A provider's logo in the 24 pt white circle of a waiting card, at 62 %.
private struct ProviderMark: View {
    var provider: String

    var body: some View {
        Group {
            if let p = Providers.all.first(where: { $0.id == provider }) {
                ProviderAvatar(provider: p, size: 24)
            } else {
                Circle().fill(.white).frame(width: 24, height: 24)
            }
        }
        // A white circle on every field, in both themes.
        .environment(\.colorScheme, .light)
    }
}

// MARK: Field

/// A field at the top of the screen, running under the status bar and the Dynamic Island, its bottom corners rounded.
private struct FieldPage: View {
    var body: some View {
        SignalField(kind: .push) {
            VStack(alignment: .leading, spacing: 12) {
                HStack(spacing: 10) {
                    RoundHeaderButton(glyph: .close, label: "Close", onField: true, size: 34) {}
                    Text("Claude Code · MacBook").textRole(.row)
                    Spacer()
                    TimerPill(createdAt: Int64(Date().timeIntervalSince1970) - 20, waitUntil: Int64(Date().timeIntervalSince1970) + 40, onField: true)
                }
                Text("GitHub · Push").textRole(.section).foregroundStyle(Palette.onField2).padding(.top, 34)
                AskText("Push 8 commits to", target: "octo/app")
                HStack(spacing: 6) {
                    GlyphIcon(.autopilot, size: 16)
                    Text("Autopilot would allow · 97%").font(RFont.sans(13.5))
                }
                .foregroundStyle(Palette.onField2)
            }
            .padding(.horizontal, 22)
            .padding(.top, 8)
            .padding(.bottom, 22)
        }
    }
}

private struct FieldNotes: View {
    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            Label("A drill-in keeps a band")
            SignalField(kind: .push, band: true) {
                HStack(spacing: 10) {
                    RoundHeaderButton(glyph: .chevronLeft, label: "Back", onField: true, size: 30) {}
                    Text("Push 8 commits to \(Text("octo/app").font(RFont.display(14, target: true)))").font(RFont.display(14))
                    Spacer()
                    Text("0:31").font(RFont.mono(13))
                }
                .padding(.horizontal, 18)
                .padding(.top, 12)
            }
            .padding(.horizontal, -24)
            Label("Facts under the field")
            VStack(spacing: 0) {
                HairlineFact(key: "Branch", value: "feature/passkeys", mono: true) { Pill("New", tone: .success) }
                HairlineFact(key: "Account", value: "octo-dana", mono: true)
                HairlineFact(key: "8 commits", value: nil, chevron: true) {
                    Text("\(Text("+412").foregroundStyle(Palette.success)) \(Text("−96").foregroundStyle(Palette.danger))").font(RFont.mono(13))
                }
            }
            Label("Allow for")
            ChooserDemo()
            RatioRow(spacing: 10, ratio: 1 / 1.35) {
                Button("Deny") {}.buttonStyle(SignalButtonStyle(.outline))
                Button {} label: { SwiftUI.Label { Text("Approve") } icon: { GlyphIcon(.faceId) } }
                    .buttonStyle(SignalButtonStyle(.inverse))
            }
            .frame(height: 54)
        }
    }
}

// MARK: Glyphs

private struct GlyphsPage: View {
    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            Text("Glyphs").textRole(.title).foregroundStyle(Palette.ink).padding(.top, 8)
            Text("\(Glyph.allCases.count) glyphs · 24 grid · 1.6 stroke").textRole(.meta).foregroundStyle(Palette.inkMeta)
            LazyVGrid(columns: [GridItem(.adaptive(minimum: 74), spacing: 6)], spacing: 14) {
                ForEach(Glyph.allCases, id: \.self) { glyph in
                    VStack(spacing: 6) {
                        GlyphIcon(glyph, size: 24).foregroundStyle(Palette.ink)
                        Text(glyph.rawValue).font(RFont.mono(9.5)).foregroundStyle(Palette.inkMeta).lineLimit(1).minimumScaleFactor(0.7)
                    }
                }
            }
            Label("On a field")
            HStack(spacing: 14) {
                ForEach([Glyph.send, .search, .trash, .key, .link, .person, .warning, .shieldCheckFilled], id: \.self) {
                    GlyphIcon($0, size: 22)
                }
            }
            .foregroundStyle(Palette.onField)
            .padding(16)
            .frame(maxWidth: .infinity)
            .fieldBackground(.grant, in: RoundedRectangle(cornerRadius: 20, style: .continuous))
        }
    }
}

#endif
