import SwiftUI

// The ask as a field (Reins Signal, iOS-Approve, iOS-Kinds): a solid colour running into the top of the screen with
// who asks and how long it waits, the label line ("GITHUB · PUSH"), the ask in the serif with its target in italic,
// and under it every warning the request carries, each on a line of its own, then Autopilot's opinion. It replaces the
// old washed hero and the agent bar above it.

/// The ask in words (pure, unit-tested): the label line and the sentence, built from the hero's own sentence.
enum AskWords {
    /// "Push 8 commits to octo/app" and its target "octo/app"; a question from the computer stays the question.
    static func ask(_ view: ApprovalView) -> (text: String, target: String?) {
        let spans = ApprovalWords.sentence(view)
        var plain = spans.map(\.text).joined().replacingOccurrences(of: "\n", with: " ")
        while plain.contains("  ") { plain = plain.replacingOccurrences(of: "  ", with: " ") }
        plain = plain.trimmingCharacters(in: .whitespaces)
        if view.ask == nil {
            if plain.hasPrefix("Wants to ") {
                plain = String(plain.dropFirst("Wants to ".count))
            } else if plain.hasPrefix("Wants ") {
                plain = "Hand over " + plain.dropFirst("Wants ".count)
            }
        }
        if let first = plain.first { plain = first.uppercased() + plain.dropFirst() }
        let bold = spans.filter(\.bold)
        var target: String?
        if view.ask == nil, bold.count >= 2 {
            let last = bold[bold.count - 1].text.replacingOccurrences(of: "\n", with: " ").trimmingCharacters(in: .whitespaces)
            if !last.isEmpty, plain.contains(last) { target = last }
        }
        // A resource's own label ("Branch main of octo/app") reads on in the sentence: "… to GitHub branch main of …".
        if let t = target, view.git == nil, view.resources.contains(where: { untrusted($0.label) == t }), let range = plain.range(of: t, options: .backwards) {
            let lowered = ApprovalWords.lowerFirst(t)
            plain.replaceSubrange(range, with: lowered)
            target = lowered
        }
        return (plain, target)
    }

    /// The ask as the serif's runs: the target (its last occurrence) in italic. A slash never ends a line ("octo/ app").
    static func runs(_ view: ApprovalView) -> [AskText.Run] {
        let (text, target) = ask(view)
        let runs: [AskText.Run]
        if let target, let range = text.range(of: target, options: .backwards) {
            runs = [
                AskText.Run(text: String(text[..<range.lowerBound])),
                AskText.Run(text: target, target: true),
                AskText.Run(text: String(text[range.upperBound...])),
            ].filter { !$0.text.isEmpty }
        } else {
            runs = [AskText.Run(text: text)]
        }
        return runs.map { AskText.Run(text: $0.text.replacingOccurrences(of: "/", with: "/\u{2060}"), target: $0.target) }
    }

    /// A long ask (a question from the computer, a long command) is set smaller, never cut.
    static func role(_ view: ApprovalView) -> TextRole { ask(view).text.count > 80 ? .sheetTitle : .ask }

    /// The label line over the ask: what it touches, then what it does ("GitHub · Push", "Payments · Buy").
    static func label(_ view: ApprovalView) -> String {
        let where_: String
        if let mcp = view.mcp {
            where_ = untrustedName(mcp.serverName)
        } else if view.purchase != nil {
            where_ = "Payments"
        } else if view.ask != nil {
            where_ = "Computer"
        } else if view.secrets != nil || ApprovalWords.isSecrets(view) {
            where_ = "Vault"
        } else if view.ssh != nil {
            where_ = "SSH"
        } else {
            where_ = serviceName(view.service.isEmpty ? "gmail" : view.service)
        }
        return "\(where_) · \(what(view))"
    }

    private static func what(_ view: ApprovalView) -> String {
        if view.mcp != nil { return "MCP tool" }
        if view.purchase != nil { return "Buy" }
        if view.ask != nil { return "Asks" }
        if view.secrets != nil || view.envRelease != nil { return "Secrets" }
        if view.secretsStore != nil || view.tokenStore != nil { return "Save" }
        if view.ssh != nil { return "Sign in" }
        if view.kind == .grant { return "Permission" }
        if view.kind == .accounts { return "Accounts" }
        if ApprovalWords.deletes(view) { return "Delete" }
        if view.git != nil { return "Push" }
        switch view.kind {
        case .send: return "Send"
        case .search: return "Search"
        case .read: return "Read"
        case .fetch: return "Fetch"
        default: break
        }
        switch ApprovalWords.badge(view) {
        case .search: return "Search"
        case .read: return "Read"
        case .list: return "List"
        case .send: return "Send"
        case .trash: return "Delete"
        case .terminal: return "Run"
        default: return "Change"
        }
    }
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
        if !tags.contains(where: \.destructive), let why = alwaysAskReason(view) {
            lines.append(FieldLine(id: "alwaysAsks", text: "Always asks · \(why)", tone: view.purchase != nil ? .red : .plain))
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

/// The approval's field: close, who asks and the timer; the label line; the ask; the warning lines and Autopilot.
struct ApprovalField: View {
    @Bindable var vm: ApprovalModel
    var view: ApprovalView
    var autopilot: AutopilotModel
    /// The status bar's height, when the field sits in a scroll that runs under it.
    var top: CGFloat = 0
    @Environment(AppModel.self) private var model

    var body: some View {
        SignalField(kind: ApprovalWords.tone(view).field) {
            VStack(alignment: .leading, spacing: 0) {
                whoAsks
                Text(AskWords.label(view))
                    .textRole(.label)
                    .foregroundStyle(Palette.onField2)
                    .lineLimit(1)
                    .padding(.top, 30)
                    .accessibilityIdentifier("fieldLabel")
                AskText(AskWords.runs(view), role: AskWords.role(view), lineLimit: nil)
                    .foregroundStyle(Palette.onField)
                    .padding(.top, 10)
                    .accessibilityElement(children: .ignore)
                    .accessibilityLabel(AskWords.ask(view).text)
                    .accessibilityAddTraits(.isHeader)
                    // A question is answered, so it is the question itself.
                    .accessibilityIdentifier(view.ask != nil ? "askQuestion" : "what")
                FieldVerdicts(vm: vm, view: view, autopilot: autopilot).padding(.top, 16)
            }
            .padding(.horizontal, 22)
            .padding(.top, top + 4)
            .padding(.bottom, 22)
        }
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("hero")
    }

    private var whoAsks: some View {
        let host = findConnection(model.connections, id: view.connectionId, label: view.connectionLabel).map { untrustedName($0.clientHost) }
        return HStack(spacing: 10) {
            RoundHeaderButton(glyph: .close, label: "Close", onField: true, size: 36) { model.closeSheet() }
                .accessibilityIdentifier("closeSheet")
            ConnectionIcon(connectionId: view.connectionId, label: view.connectionLabel, size: 28)
            (Text(verbatim: untrustedName(view.connectionLabel)).foregroundStyle(Palette.onField)
                + Text(verbatim: host.map { $0.isEmpty ? "" : " · \($0)" } ?? "").foregroundStyle(Palette.onField2))
                .font(RFont.sans(14.5, .medium))
                .lineLimit(1)
                .frame(maxWidth: .infinity, alignment: .leading)
                .accessibilityElement(children: .combine)
            FieldTimer(view: view)
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
                            Text("Stopped").font(RFont.sans(13.5, .semibold))
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

/// Under the ask: every warning line, then (unless it is held) Autopilot's opinion, why it always asks, or the offer
/// to get its model.
private struct FieldVerdicts: View {
    @Bindable var vm: ApprovalModel
    var view: ApprovalView
    var autopilot: AutopilotModel
    @Environment(ApprovalNav.self) private var nav
    @Environment(\.feedback) private var feedback

    var body: some View {
        let s = vm.suggestion
        VStack(alignment: .leading, spacing: 9) {
            ForEach(FieldLines.of(view, suggestion: s, risk: s?.risk, riskInFacts: nav.riskInFacts)) { line in
                HStack(alignment: .center, spacing: 8) {
                    FieldVerdictRow(
                        text: line.text, glyph: line.symbol, strong: line.tone == .red, lineLimit: line.maxLines,
                        action: line.id == "risk" && s?.judged == true ? openAutopilot : nil,
                        onOverflow: line.id == "risk" ? { if $0 && !nav.riskInFacts { nav.riskInFacts = true } } : nil
                    )
                    .accessibilityIdentifier(line.id)
                    if let help = line.help { HelpDot(id: line.id, text: help, size: 26, onField: true) }
                }
            }
            if !ApprovalWords.held(view) { autopilotLine(s) }
        }
    }

    @ViewBuilder private func autopilotLine(_ s: SuggestionView?) -> some View {
        if let s, s.judged, !s.floor {
            let p = s.verdict == .deny ? s.pDeny : s.pApprove
            let words = switch s.verdict {
            case .approve: "Autopilot would allow"
            case .deny: "Autopilot would deny"
            case .ask: "Autopilot is unsure"
            }
            FieldVerdictRow(
                text: "\(words) · \(SuggestionText.percent(p))", glyph: s.verdict == .ask ? nil : .autopilot,
                ring: s.verdict == .ask ? Double(p) : nil, chevron: true, action: openAutopilot
            )
            .accessibilityIdentifier("suggestion")
        } else if let model = autopilot.model, !AutopilotText.modelUsable(model), s?.floor != true {
            FieldDownloadLine(autopilot: autopilot, model: model)
        } else if let s, s.floor, ApprovalWords.dangerTags(view).isEmpty, FieldLines.alwaysAskReason(view) == nil {
            FieldVerdictRow(text: "Always asks", glyph: .lock, chevron: true, action: openAutopilot)
                .accessibilityIdentifier("suggestion")
        }
    }

    private func openAutopilot() {
        nav.overlay = .autopilot
        feedback.play(.open)
    }
}

/// A line on a field: a glyph (or a ring) and a few words in on-field ink; a tap opens its detail.
struct FieldVerdictRow: View {
    var text: String
    var glyph: Glyph? = nil
    var ring: Double? = nil
    /// A red line: full white and semibold.
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
            Button(action: action) { row }.buttonStyle(.plain)
        } else {
            row
        }
    }

    private var row: some View {
        let ink = strong ? Palette.onField : Palette.onField2
        return HStack(alignment: .top, spacing: 8) {
            if let ring {
                ArcRing(fraction: ring, color: Palette.onField, track: Palette.onField.opacity(0.3), lineWidth: 2.2)
                    .frame(width: 16, height: 16)
                    .padding(.top, 1.5)
            } else if let glyph {
                GlyphIcon(glyph, size: 16).foregroundStyle(ink).padding(.top, 1.5)
            }
            label
                .font(RFont.sans(14.5, strong ? .semibold : .regular))
                .foregroundStyle(ink)
                .lineLimit(lineLimit)
                .fixedSize(horizontal: false, vertical: true)
                .onGeometryChange(for: CGFloat.self, of: \.size.height) { shown = $0; report() }
                .background(alignment: .topLeading) {
                    if onOverflow != nil {
                        // The same words uncut, unseen: whether the lines above cut them.
                        label
                            .font(RFont.sans(14.5, strong ? .semibold : .regular))
                            .fixedSize(horizontal: false, vertical: true)
                            .hidden()
                            .onGeometryChange(for: CGFloat.self, of: \.size.height) { full = $0; report() }
                    }
                }
            if chevron { GlyphIcon(.chevronRight, size: 14).foregroundStyle(ink).padding(.top, 2.5) }
        }
        .contentShape(Rectangle())
        .accessibilityElement(children: .combine)
    }

    private var label: some View {
        Text(verbatim: text).lineHeight(.multiple(factor: 19 / 14.5))
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
                FieldVerdictRow(text: pct.map { "Downloading Autopilot · \($0)%" } ?? "Downloading Autopilot…", glyph: .download)
                    .accessibilityIdentifier("modelProgress")
            case (_, .waiting):
                FieldVerdictRow(text: autopilot.waitingForWifi ? "Autopilot waits for Wi-Fi" : "Autopilot waits for a network", glyph: .download)
                    .accessibilityIdentifier("modelProgress")
            default:
                FieldVerdictRow(text: "Get Autopilot's score", glyph: .autopilot)
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
