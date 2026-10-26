import SwiftUI

// The ask as a field (Reins Next: Next-Approve, Next-Kinds, Next-Running "D · the ask, final"): a colour field
// running into the top of the screen, the timer and close in the island's ears, the service's icon · who asks on one
// short line, the ask in `.ask` with its target in 600 and its machine details in mono under it (AskFormat) beside
// Autopilot's pick, then every warning the request carries, each a flag of its own.

extension AskLine {
    /// A long ask (a question from the computer in its own words) is set smaller, never cut.
    var role: TextRole { text.count > 80 ? .cardAsk : .ask }
}

/// One warning line on the field: its own words, how it reads, and (for some) a longer reason behind its (?).
struct FieldLine: Equatable, Identifiable {
    enum Tone: Equatable {
        /// Held and asked every time: white, with a lock.
        case red
        /// Something to check: the warning sign.
        case amber
        /// Said for the record.
        case plain
    }

    /// Its accessibility identifier, the tag's own ("forceWarning:0", "risk").
    var id: String
    var text: String
    var tone: Tone
    var help: String? = nil
    /// Nil: never cut. The risk wraps to two lines, then gives its reason to the first fact row.
    var maxLines: Int? = nil
    var glyph: Glyph? = nil

    var symbol: Glyph {
        if let glyph { return glyph }
        switch tone {
        case .red: return .lock
        case .amber: return .warning
        case .plain: return .shieldAlert
        }
    }
}

/// Every warning on the field (pure, unit-tested). Nothing a request carries is behind a fold or only a (?).
enum FieldLines {
    /// In this order: each danger tag (the guard rule, the computer's own rule, a command this phone couldn't read, a
    /// push that rewrites history or wasn't checked, hidden characters, the default branch, a destructive tool, what
    /// can't be undone); an SSH server with no name; why it is asked every time, when no red line says so; the first
    /// time this AI asks for this; the risk Overnight's model saw (two lines; its word only once the reason went to the
    /// facts), and that it held it.
    static func of(_ view: ApprovalView, suggestion: SuggestionView?, risk: RiskView?, riskInFacts: Bool = false) -> [FieldLine] {
        var lines: [FieldLine] = []
        let tags = ApprovalWords.dangerTags(view)
        for tag in tags {
            lines.append(FieldLine(id: tag.id, text: line(tag), tone: tag.destructive ? .red : .amber, help: tag.help))
        }
        if let ssh = view.ssh, (ssh.host ?? "").trimmingCharacters(in: .whitespaces).isEmpty {
            let keyed = !(ssh.hostKey ?? "").trimmingCharacters(in: .whitespaces).isEmpty
            lines.append(FieldLine(id: "unknownHost", text: keyed ? "Server known only by its key" : "Unknown server", tone: .amber))
        }
        if !tags.contains(where: \.destructive) {
            if ApprovalWords.mailPermissionSettings(view) { lines.append(FieldLine(id: "alwaysAsks", text: "This approval is once only. Manage ongoing access in Trust.", tone: .plain)) }
            else if let why = alwaysAskReason(view) { lines.append(FieldLine(id: "alwaysAsks", text: "Always asks · \(why)", tone: view.purchase != nil ? .red : .plain)) }
        }
        if suggestion?.novel == true {
            lines.append(FieldLine(id: "novel", text: "First time for this AI", tone: .amber, glyph: .person))
        }
        if let risk {
            let text = riskInFacts ? "Risk: \(AutopilotText.riskWord(risk.level))" : AutopilotText.riskLine(risk)
            lines.append(FieldLine(id: "risk", text: text, tone: .amber, maxLines: 2, glyph: .shieldAlert))
            if risk.held { lines.append(FieldLine(id: "riskHeld", text: AutopilotText.heldLine, tone: .amber, glyph: .moon)) }
        }
        return lines
    }

    /// A red tag says it always asks and why ("Always asks · can't be undone"); an amber one says itself.
    static func line(_ tag: ApprovalWords.DangerTag) -> String {
        if tag.id == "ruleReported" { return "Your computer says so; this phone doesn't know the rule" }
        guard tag.destructive else { return tag.label }
        let words = ["onceWarning", "mcpDestructive"].contains(tag.id) ? "can't be undone" : ApprovalWords.lowerFirst(tag.label)
        return "Always asks · \(words)"
    }

    /// Why nothing here may be remembered, in a few words; nil when it may be.
    static func alwaysAskReason(_ view: ApprovalView) -> String? {
        if view.purchase != nil { return "money can't come back" }
        guard ApprovalWords.rememberLocked(view) else { return nil }
        if let env = view.envRelease {
            if env.print { return "it prints a value" }
            if env.alwaysAsk { return "your computer asks for it every time" }
            return "this command can't be remembered"
        }
        if view.ssh != nil { return "it signs in to a server" }
        if view.service == "vault" { return "it hands over a password" }
        return "it's never remembered"
    }
}

/// Where the field's top row (the timer at the left, close at the right) sits (Next-Running "D · the ask, final",
/// Next-SE option B, Next-AX). Pure, unit-tested.
enum FieldEars: Equatable {
    /// In the Dynamic Island's ears, 30 pt in from the screen's edges; the status bar steps aside.
    case island
    /// No island: the status bar stays and the row sits right under it, 16 pt in.
    case underStatusBar
    /// Accessibility text sizes: the status bar stays and the row is the field's first row.
    case firstRow

    /// `top`: the screen's top safe-area inset (59 on an island phone, 20 on an iPhone SE).
    static func of(top: CGFloat, accessibility: Bool) -> FieldEars {
        if accessibility { return .firstRow }
        return top >= 47 ? .island : .underStatusBar
    }

    var hidesStatusBar: Bool { self == .island }

    /// How far below the screen's top the field's content starts.
    func contentTop(_ top: CGFloat) -> CGFloat {
        switch self {
        case .island: max(top + 7, 66)
        case .underStatusBar: top + 48
        case .firstRow: top
        }
    }
}

/// The approval's field (the kit's ask): the timer and close in the ears, then the service's icon · who asks on one short
/// line, the ask (`.ask`, its target in 600), the machine value in mono with Autopilot's pick beside it, and every
/// warning the request carries as a flag of its own. Who asks is Reins' own line, apart from Reins' sentence; a name
/// the requester chose never joins the sentence (ASK-TEXT.md).
struct ApprovalField: View {
    @Bindable var vm: ApprovalModel
    var view: ApprovalView
    var autopilot: AutopilotModel
    /// The screen's top safe-area inset: the field runs under the status bar.
    var top: CGFloat = 0
    @Environment(AppModel.self) private var model
    @Environment(ApprovalNav.self) private var nav
    @Environment(\.feedback) private var feedback
    @Environment(\.dynamicTypeSize) private var typeSize

    var body: some View {
        let ears = FieldEars.of(top: top, accessibility: typeSize.isAccessibilitySize)
        let field = ApprovalWords.tone(view).field
        let ask = AskFormat.waiting(view, core: model.pending.first { $0.id == view.requestId }?.label, computer: computer)
        SignalField(kind: field) {
            VStack(alignment: .leading, spacing: 10) {
                if ears == .firstRow { earsRow.padding(.bottom, 4) }
                AskHead(ask: ask, field: field, askId: view.ask != nil ? "askQuestion" : "what") {
                    WhoAsks(connectionId: view.connectionId, label: view.connectionLabel, host: computer)
                } pick: {
                    FieldPick(vm: vm, view: view, open: openAutopilot)
                }
                FieldFlags(vm: vm, view: view, autopilot: autopilot, open: openAutopilot)
            }
            .padding(.horizontal, 20)
            .padding(.top, ears.contentTop(top))
            .padding(.bottom, 22)
        }
        .overlay(alignment: .top) {
            switch ears {
            case .island: earsRow.frame(height: 36).padding(.horizontal, 30).padding(.top, 12)
            case .underStatusBar: earsRow.frame(height: 36).padding(.horizontal, 16).padding(.top, top + 4)
            case .firstRow: EmptyView()
            }
        }
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("hero")
    }

    /// The timer at the left, close at the right. In the ears, each side stays clear of the island (95 pt at most).
    private var earsRow: some View {
        let ears = FieldEars.of(top: top, accessibility: typeSize.isAccessibilitySize)
        let side: CGFloat? = ears == .island ? 95 : ears == .underStatusBar ? 160 : nil
        return HStack(spacing: 8) {
            FieldTimer(view: view).frame(maxWidth: side, alignment: .leading)
            Spacer(minLength: 0)
            RoundHeaderButton(glyph: .close, label: "Close", onField: true, size: ears == .firstRow ? 44 : 32) { model.closeSheet() }
                .accessibilityIdentifier("closeSheet")
        }
    }

    /// The asking computer's name, for a command with no folder of its own.
    private var computer: String? {
        findConnection(model.connections, id: view.connectionId, label: view.connectionLabel).map { asciiHost(untrustedName($0.clientHost)) }
    }

    private func openAutopilot() {
        nav.overlay = .autopilot
        feedback.play(.open)
    }
}

/// Who asks, on the field's first line: its avatar and the name it was given (one line, cut), then the address it
/// asks from, dimmer. Never part of the sentence.
struct WhoAsks: View {
    var connectionId: String
    var label: String
    var host: String?
    @Environment(\.dynamicTypeSize) private var typeSize

    var body: some View {
        HStack(spacing: 8) {
            ConnectionIcon(connectionId: connectionId, label: label, size: 22)
            (Text(verbatim: untrustedName(label, limit: 40)).foregroundStyle(Palette.onField)
                + Text(verbatim: host.map { $0.isEmpty ? "" : " · \($0)" } ?? "").foregroundStyle(Palette.onField2))
                .font(RFont.sans(15, .semibold))
                .lineLimit(typeSize.isAccessibilitySize ? 3 : 1)
                .truncationMode(.middle)
                .environment(\.layoutDirection, .leftToRight)
        }
        .accessibilityElement(children: .combine)
        .accessibilityLabel("Asked by \(untrustedName(label, limit: 40))\(host.map { $0.isEmpty ? "" : ", \($0)" } ?? "")")
        .accessibilityIdentifier("whoAsks")
    }
}

/// The request on a field (ASK-TEXT, the kit's ask): the service's icon · who asks, the sentence in `.ask` with its
/// target in 600, never cut, and its machine details in mono under it (wrapped, never cut) with `pick` at the right.
struct AskHead<Who: View, Pick: View>: View {
    var ask: AskLine
    var field: FieldKind
    /// The sentence's identifier: "what", or "askQuestion" for a question to answer.
    var askId = "what"
    @ViewBuilder var who: Who
    @ViewBuilder var pick: Pick
    @Environment(\.dynamicTypeSize) private var typeSize

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            HStack(spacing: 8) {
                RequestIcon(spec: ask.icon, size: 26, ring: field.fill)
                    .accessibilityRepresentation { Text(verbatim: ask.kind) }
                    .accessibilityIdentifier("fieldLabel")
                if Who.self != EmptyView.self {
                    Text(verbatim: "·").font(RFont.sans(15, .semibold)).opacity(0.6).accessibilityHidden(true)
                    who
                }
            }
            AskText(ask.runs, role: ask.role, lineLimit: nil)
                .foregroundStyle(Palette.onField)
                .accessibilityElement(children: .ignore)
                .accessibilityLabel(ask.text)
                .accessibilityAddTraits(.isHeader)
                .accessibilityIdentifier(askId)
            if let mono = ask.mono {
                // At accessibility sizes the pick goes under the mono line, so neither has to wrap around the other.
                let layout = typeSize.isAccessibilitySize
                    ? AnyLayout(VStackLayout(alignment: .leading, spacing: 10)) : AnyLayout(HStackLayout(alignment: .center, spacing: 10))
                layout {
                    Text(verbatim: mono)
                        .font(TextRole.value.font())
                        .foregroundStyle(Color.white.opacity(0.85))
                        .fixedSize(horizontal: false, vertical: true)
                        .frame(maxWidth: .infinity, alignment: .leading)
                        .environment(\.layoutDirection, .leftToRight)
                        .textSelection(.enabled)
                        .accessibilityIdentifier("askMono")
                    pick
                }
            } else {
                pick
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
    }
}

extension AskHead where Who == EmptyView, Pick == EmptyView {
    /// A sheet's own ask (a pairing): no requester on the line, nothing to pick.
    init(ask: AskLine, field: FieldKind, askId: String = "what") {
        self.init(ask: ask, field: field, askId: askId, who: { EmptyView() }, pick: { EmptyView() })
    }
}

/// Autopilot's pick on the field, beside the mono line: its glyph and how sure ("97%"); a pick other than approving
/// says which. Opens why. Nothing while the request is held.
private struct FieldPick: View {
    @Bindable var vm: ApprovalModel
    var view: ApprovalView
    var open: () -> Void

    var body: some View {
        if let s = vm.suggestion, s.judged, !s.floor, !ApprovalWords.held(view) {
            let p = s.verdict == .deny ? s.pDeny : s.pApprove
            let percent = SuggestionText.percent(p)
            let (text, words) = switch s.verdict {
            case .approve: (percent, "Autopilot would allow")
            case .deny: ("Deny \(percent)", "Autopilot would deny")
            case .ask: ("Unsure \(percent)", "Autopilot is unsure")
            }
            Button(action: open) {
                // A 44 pt target around the 24 pt pill, without making its line taller.
                Pill(text, tone: .onFieldWash, mode: .auto, size: .pill)
                    .contentShape(Rectangle().inset(by: -10))
            }
            .buttonStyle(.plain)
            .accessibilityLabel("\(words) · \(percent)")
            .accessibilityIdentifier("suggestion")
        }
    }
}

/// The time the AI still waits, on the field: the white ring and mono digits (a white pill with red digits in the last
/// 15 s); "Stopped" once it gave up, whose tap says approving still works.
struct FieldTimer: View {
    var view: ApprovalView
    @State private var lateHelp = false

    var body: some View {
        UrgencyClock(createdAt: view.createdAt, waitUntil: view.waitUntil) { urgency in
            if let u = urgency {
                if u.stale {
                    Button {
                        lateHelp = true
                    } label: {
                        HStack(spacing: 6) {
                            GlyphIcon(.timerOff, size: 16)
                            Text("Stopped").font(RFont.sans(13.5, .semibold)).lineLimit(1)
                        }
                        .foregroundStyle(Palette.onField)
                        .padding(.leading, 9)
                        .padding(.trailing, 11)
                        .padding(.vertical, 6)
                        .background(Palette.onFieldWash, in: Capsule())
                    }
                    .buttonStyle(.plain)
                    .accessibilityIdentifier("lateBanner")
                    .popover(isPresented: $lateHelp) {
                        HelpText(text: "\(untrustedName(view.connectionLabel)) stopped waiting. You can still approve; then ask it to try again.")
                    }
                } else {
                    TimerPill(createdAt: view.createdAt, waitUntil: view.waitUntil, onField: true)
                        .accessibilityIdentifier("waitLine")
                }
            }
        }
    }
}

/// Under the ask: every warning as a flag of its own (never behind a fold), then why it always asks, or the offer to
/// get Autopilot's model.
private struct FieldFlags: View {
    @Bindable var vm: ApprovalModel
    var view: ApprovalView
    var autopilot: AutopilotModel
    var open: () -> Void
    @Environment(ApprovalNav.self) private var nav

    var body: some View {
        let s = vm.suggestion
        let lines = FieldLines.of(view, suggestion: s, risk: s?.risk, riskInFacts: nav.riskInFacts)
        let tail = !ApprovalWords.held(view) ? tailKind(s) : nil
        if !lines.isEmpty || tail != nil {
            VStack(alignment: .leading, spacing: 8) {
                ForEach(lines) { line in
                    HStack(alignment: .center, spacing: 6) {
                        FieldFlag(
                            text: line.text, glyph: line.symbol, strong: line.tone == .red, lineLimit: line.maxLines,
                            action: line.id == "risk" && s?.judged == true ? open : nil,
                            onOverflow: line.id == "risk" ? { if $0 && !nav.riskInFacts { nav.riskInFacts = true } } : nil
                        )
                        .accessibilityIdentifier(line.id)
                        if let help = line.help { HelpDot(id: line.id, text: help, size: 26, onField: true) }
                    }
                }
                switch tail {
                case .download(let model): FieldDownloadLine(autopilot: autopilot, model: model)
                case .floor:
                    FieldFlag(text: "Always asks", glyph: .lock, chevron: true, action: open)
                        .accessibilityIdentifier("suggestion")
                case nil: EmptyView()
                }
            }
        }
    }

    private enum Tail { case download(ModelStatus), floor }

    private func tailKind(_ s: SuggestionView?) -> Tail? {
        if let s, s.judged, !s.floor { return nil }
        if let model = autopilot.model, !AutopilotText.modelUsable(model), s?.floor != true { return .download(model) }
        if let s, s.floor, ApprovalWords.dangerTags(view).isEmpty, FieldLines.alwaysAskReason(view) == nil { return .floor }
        return nil
    }
}

/// A flag on a field (the kit's on-field pill): a glyph (or a ring) and a few words on the white wash, wrapping rather
/// than cut unless `lineLimit` says so; a tap opens its detail.
struct FieldFlag: View {
    var text: String
    var glyph: Glyph? = nil
    var ring: Double? = nil
    /// A red line: on solid white, in the field's own ink, so it reads first.
    var strong = false
    var lineLimit: Int? = nil
    var chevron = false
    var action: (() -> Void)? = nil
    /// Told whether the text needed more lines than it has.
    var onOverflow: ((Bool) -> Void)? = nil
    @State private var shown: CGFloat = 0
    @State private var full: CGFloat = 0

    var body: some View {
        if let action {
            Button(action: action) { flag }.buttonStyle(.plain)
        } else {
            flag
        }
    }

    private var flag: some View {
        HStack(alignment: .top, spacing: 5) {
            if let ring {
                ArcRing(fraction: ring, color: Palette.onField, track: Palette.onField.opacity(0.3), lineWidth: 2)
                    .frame(width: 13, height: 13)
                    .padding(.top, 2.5)
            } else if let glyph {
                GlyphIcon(glyph, size: 13).padding(.top, 2.5)
            }
            label
                .lineLimit(lineLimit)
                .fixedSize(horizontal: false, vertical: true)
                .onGeometryChange(for: CGFloat.self, of: \.size.height) { shown = $0; report() }
                .background(alignment: .topLeading) {
                    if onOverflow != nil {
                        // The same words uncut, unseen: whether the lines above cut them.
                        label.fixedSize(horizontal: false, vertical: true).hidden()
                            .onGeometryChange(for: CGFloat.self, of: \.size.height) { full = $0; report() }
                    }
                }
            if chevron { GlyphIcon(.chevronRight, size: 12).padding(.top, 3).opacity(0.8) }
        }
        .foregroundStyle(Palette.onField)
        .padding(.horizontal, 9)
        .padding(.vertical, 4)
        .frame(minHeight: 24)
        .background(Color.white.opacity(strong ? 0.3 : 0.2), in: RoundedRectangle(cornerRadius: 12, style: .continuous))
        .contentShape(Rectangle())
        .accessibilityElement(children: .combine)
    }

    private var label: some View {
        Text(verbatim: text)
            .font(RFont.sans(12.5, .semibold))
            .lineHeight(.multiple(factor: 17 / 12.5))
    }

    private func report() {
        guard let onOverflow, shown > 0, full > 0 else { return }
        onOverflow(full > shown + 1)
    }
}

/// No model on this phone: "Get Autopilot's score" and a white Download; while it downloads, how far.
private struct FieldDownloadLine: View {
    var autopilot: AutopilotModel
    var model: ModelStatus
    @State private var askMobileData = false
    @Environment(\.feedback) private var feedback

    var body: some View {
        HStack(spacing: 10) {
            switch (model.state, autopilot.downloadJob) {
            case (.downloading, _):
                let pct = model.sizeBytes > 0 ? Int(model.downloadedBytes * 100 / model.sizeBytes) : nil
                FieldFlag(text: pct.map { "Downloading Autopilot · \($0)%" } ?? "Downloading Autopilot…", glyph: .download)
                    .accessibilityIdentifier("modelProgress")
            case (_, .waiting):
                FieldFlag(text: autopilot.waitingForWifi ? "Autopilot waits for Wi-Fi" : "Autopilot waits for a network", glyph: .download)
                    .accessibilityIdentifier("modelProgress")
            default:
                FieldFlag(text: "Get Autopilot's score", glyph: .autopilot)
                Button("Download") {
                    feedback.play(.selection)
                    if autopilot.needsMobileDataConsent() { askMobileData = true } else { autopilot.download() }
                }
                .buttonStyle(SignalButtonStyle(.onField, size: .small))
                .accessibilityIdentifier("downloadModel")
            }
        }
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("getAutopilot")
        .confirmationDialog("Download over mobile data?", isPresented: $askMobileData, titleVisibility: .visible) {
            Button("Download now") { autopilot.downloadNow() }
            Button("Wait for Wi-Fi") { autopilot.download() }
        }
    }
}
