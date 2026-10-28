import SwiftUI

// The Autopilot parts of the Activity screens (the Android app's ActivityAutopilot.kt): the modes in words, an entry's
// outcome pill, "Why Autopilot allowed it", the "This was wrong" dock, the matrix's line and "Ask me next time". The
// header's mode chip is ActivityModeMenu.

/// Autopilot's modes in words, colours and symbols, as these screens show them.
enum ModeLook {
    static func name(_ mode: AutopilotMode) -> String {
        switch mode {
        case .manual: "Manual"
        case .auto: "Auto"
        case .overnight: AutopilotText.overnightName
        case .bypass: "Bypass"
        case .lockdown: "Lockdown"
        }
    }

    static func tint(_ mode: AutopilotMode) -> Color {
        switch mode {
        case .manual: Palette.kindRead
        case .auto: Palette.violet
        case .overnight: Palette.violet
        case .bypass: Palette.danger
        case .lockdown: Palette.warning
        }
    }

    static func symbol(_ mode: AutopilotMode) -> String {
        switch mode {
        case .manual: "person.fill.checkmark"
        case .auto: "bolt"
        case .overnight: AutopilotText.overnightSymbol
        case .bypass: "bolt.fill"
        case .lockdown: "lock.fill"
        }
    }

    /// "14:05" left, "2d 4h" beyond a day, "∞" for a bypass without an end.
    static func clock(until: Int64, now: Int64) -> String { AutopilotText.clock(until: until, now: now) }

    static func verdictWord(_ v: Verdict) -> String {
        switch v {
        case .approve: "Approve"
        case .deny: "Deny"
        case .ask: "Ask you"
        }
    }
}

extension ModeLook {
    /// A bypass or an Overnight runs: the pill turns solid in its colour and counts down.
    static func running(_ settings: AutopilotSettings) -> Bool { runningUntil(settings) != nil }

    /// The end of the bypass or Overnight in force (`AutopilotText.untilOff` without one), nil otherwise.
    static func runningUntil(_ settings: AutopilotSettings) -> Int64? {
        switch settings.mode {
        case .bypass: settings.bypassUntil
        case .overnight: settings.overnightUntil
        default: nil
        }
    }

    /// What reads on the mode's solid colour: white on Bypass's red, and on Overnight's indigo a deep night in dark mode,
    /// where the indigo is light.
    static func onTint(_ mode: AutopilotMode) -> Color {
        mode == .overnight ? Palette.dynamic(0xFFFFFF, 0x0E1033) : Color.white
    }
}


/// What should have happened instead of what did: a denial was a mistaken approval's correction, and the reverse.
func correctionFor(_ entry: ActivityEntry) -> Verdict { entry.outcome == "denied" ? .approve : .deny }

/// The outcome pill under an entry's sentence ("Allowed by Autopilot · 96%"), in words, symbol and colour (pure,
/// unit-tested).
enum EntryOutcome {
    static func permissionActor(_ entry: ActivityEntry) -> String? {
        switch entry.decidedBy {
        case "account_permission": "account permission"
        case "temporary_permission": "temporary permission"
        default: nil
        }
    }

    static func text(_ entry: ActivityEntry) -> String {
        switch entry.outcome {
        case "started": return "Awaiting outcome"
        case "unknown": return "Outcome unknown · do not repeat"
        case "succeeded_withheld": return "Completed; response withheld · do not repeat"
        case "error": return "Failed"
        default: break
        }
        // A held email has not gone (08 R10), whoever approved it.
        if let held = HoldText.outcome(entry.outcome, hold: entry.hold) { return held }
        let denied = entry.outcome == "denied"
        if let actor = permissionActor(entry) { return denied ? "Denied · \(actor)" : "Allowed by \(actor)" }
        switch entry.decidedBy {
        case "autopilot":
            let p = entry.autopilot.map { denied ? $0.pDeny : $0.pApprove }
            let base = denied ? "Denied by Autopilot" : "Allowed by Autopilot"
            return p.map { $0 > 0 ? "\(base) · \(SuggestionText.percent($0))" : base } ?? base
        case "bypass": return "Allowed during a bypass"
        case "overnight": return denied ? "Denied by \(AutopilotText.overnightName)" : "Allowed by \(AutopilotText.overnightName)"
        case "lockdown": return "Denied by Lockdown"
        default: break
        }
        if !entry.decidedBy.isEmpty { return denied ? "Denied" : entry.outcome == "granted" ? "Access given" : "Allowed" }
        if entry.grantId != nil && !denied { return "Covered by a grant" }
        switch entry.outcome {
        case "denied": return "Denied by you"
        case "error": return "Failed"
        case "granted": return "Granted by you"
        case "sent": return "Sent"
        default: return "Allowed by you"
        }
    }

    static func symbol(_ entry: ActivityEntry) -> String {
        switch EntryText.mark(entry) {
        case .auto: "bolt"
        case .overnight: AutopilotText.overnightSymbol
        case .bypass: "bolt.fill"
        case .lockdown: "lock.fill"
        case .grant, .permission: "key.horizontal"
        case .denied: "xmark"
        case .failed, .started, .unknown, .withheld, .notSent: "exclamationmark.triangle"
        case .approved: "checkmark"
        case .held: "paperplane"
        case .undone: "arrow.uturn.backward"
        }
    }

    static func tint(_ entry: ActivityEntry) -> Color {
        switch EntryText.mark(entry) {
        case .auto, .grant, .permission: Palette.violet
        case .overnight: Palette.violet
        case .bypass, .denied: Palette.danger
        case .lockdown, .failed, .started, .unknown, .withheld, .notSent: Palette.warning
        case .approved: Palette.success
        case .held: Palette.violet
        case .undone: Palette.ink2
        }
    }

    /// The row that opens Autopilot's reasons: "Why Autopilot allowed it", "Why Autopilot denied it", "What Autopilot
    /// suggested".
    static func whyTitle(_ entry: ActivityEntry) -> String {
        if ["started", "unknown", "succeeded_withheld", "error"].contains(entry.outcome) { return "What Autopilot suggested" }
        return
        switch entry.decidedBy {
        case "autopilot", "overnight": entry.outcome == "denied" ? "Why Autopilot denied it" : "Why Autopilot allowed it"
        default: "What Autopilot suggested"
        }
    }
}

/// How an entry ended, as a pill: its symbol and words in the outcome's colour ("Denied by you", "Allowed by
/// Autopilot · 96%"). The entry's hero shows it, and so does the approval's hero when an entry is shown with it.
struct EntryOutcomePill: View {
    var text: String
    var symbol: String
    var tint: Color

    var body: some View {
        HStack(spacing: 6) {
            Image(systemName: symbol).font(.system(size: 12, weight: .semibold))
            Text(text).font(RFont.sans(13, .semibold)).lineLimit(1)
        }
        .foregroundStyle(tint)
        .padding(.horizontal, 12)
        .padding(.vertical, 6)
        .background(tint.opacity(0.16), in: Capsule())
        .accessibilityElement(children: .combine)
        .accessibilityIdentifier("entryOutcome")
    }
}

extension EntryOutcomePill {
    init(entry: ActivityEntry) {
        self.init(text: EntryOutcome.text(entry), symbol: EntryOutcome.symbol(entry), tint: EntryOutcome.tint(entry))
    }
}

extension ApprovalRecord {
    /// How `entry` ended, for the approval's hero, with the risk the model saw in it.
    static func of(_ entry: ActivityEntry) -> ApprovalRecord {
        ApprovalRecord(outcome: EntryOutcome.text(entry), symbol: EntryOutcome.symbol(entry), tint: EntryOutcome.tint(entry),
                       risk: entry.autopilot?.risk, label: entry.label)
    }
}

/// A risk level's colour and symbol: low reads as fine, then amber, orange and red.
enum RiskLook {
    static func tint(_ level: RiskLevel) -> Color {
        switch level {
        case .benign: Palette.success
        case .risky: Palette.warning
        case .severe: Palette.kindPush
        case .catastrophic: Palette.danger
        }
    }

    static func symbol(_ level: RiskLevel) -> String {
        switch level {
        case .benign: "checkmark.shield"
        case .risky: "exclamationmark.shield"
        case .severe: "exclamationmark.triangle.fill"
        case .catastrophic: "xmark.octagon.fill"
        }
    }

    /// Worth a line in the history: anything above low, and whatever Overnight held.
    static func listed(_ risk: RiskView?) -> Bool {
        guard let risk else { return false }
        return risk.held || risk.level != .benign
    }
}

/// "Risk: severe · force push" in the level's colour, and, when Overnight held the request, "Held by Overnight" with
/// the moon. Shown only where the model rated the risk.
struct RiskChips: View {
    var risk: RiskView

    var body: some View {
        let level = WarnTag(text: AutopilotText.riskLine(risk), tint: RiskLook.tint(risk.level), symbol: RiskLook.symbol(risk.level))
            .truncationMode(.tail)
            .accessibilityIdentifier("risk")
        if risk.held {
            let held = WarnTag(text: AutopilotText.heldLine, tint: Palette.violet, symbol: AutopilotText.overnightSymbol)
                .accessibilityIdentifier("riskHeld")
            ViewThatFits(in: .horizontal) {
                HStack(spacing: 8) { level; held }
                VStack(spacing: 8) { level; held }
            }
        } else {
            level
        }
    }
}

/// "Why Autopilot denied ›" (Next-Activity, 06): a fact row in Autopilot's violet that opens its reasons in a sheet.
/// Only for entries Autopilot looked at.
struct AutopilotWhyRow: View {
    var note: AutopilotNote
    var title: String
    /// The entry it is about, for the reason line worked out on the phone.
    var entry: ActivityEntry? = nil
    @State private var open = false
    @Environment(\.feedback) private var feedback
    @Environment(AppModel.self) private var model

    var body: some View {
        Button {
            open = true
            feedback.play(.tap)
        } label: {
            HStack(spacing: 12) {
                ModeGlyph(.auto, size: 18).foregroundStyle(Palette.violet).frame(width: 20)
                Text(AutopilotWhySheet.heading(title)).font(RFont.sans(15.5, .medium)).foregroundStyle(Palette.ink).lineLimit(2)
                    .frame(maxWidth: .infinity, alignment: .leading)
                RowChevron()
            }
            .padding(.horizontal, 16)
            .frame(minHeight: 50)
            .contentShape(Rectangle())
        }
        .buttonStyle(RowButtonStyle())
        .accessibilityIdentifier("entryAutopilotTitle")
        .task(id: entry?.id) {
            if let entry { await model.reasons.explain(entry, model: model) }
        }
        #if DEBUG
        // `-openWhy` (screenshots): the reasons open with the page.
        .onAppear { if ProcessInfo.processInfo.arguments.contains("-openWhy") { open = true } }
        #endif
        .sheet(isPresented: $open) {
            AutopilotWhySheet(note: note, title: title, entry: entry) { open = false }
                .environment(\.feedback, feedback)
        }
        .presentationFeedback(open)
    }
}

/// Why Autopilot decided (Next-Activity, 07): its verdict and how sure it was, its reasons as facts (the risk it saw,
/// its own reason, the decisions of yours it was like, the profile), and last, quietly, "This was wrong" for an entry
/// the core says can still be corrected.
struct AutopilotWhySheet: View {
    var note: AutopilotNote
    var title: String
    var entry: ActivityEntry?
    var close: () -> Void
    @Environment(AppModel.self) private var model
    @State private var height: CGFloat = 420
    @State private var correctionShown = false

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 0) {
                HStack(spacing: 12) {
                    Text(Self.heading(title))
                        .textRole(.cardAsk)
                        .foregroundStyle(Palette.ink)
                        .accessibilityAddTraits(.isHeader)
                        .frame(maxWidth: .infinity, alignment: .leading)
                    RoundHeaderButton(glyph: .close, label: "Close") { close() }
                }
                .padding(.horizontal, 20)
                .padding(.top, 20)
                verdict
                    .padding(.horizontal, 20)
                    .padding(.top, 2)
                SheetGroup {
                    ForEach(Array(reasons.enumerated()), id: \.offset) { _, r in
                        GlyphFact(glyph: r.glyph, value: r.text, tint: r.tint)
                    }
                }
                .padding(.top, 14)
                .accessibilityIdentifier("autopilotReason")
                // Stays once shown, so its thanks can replace it after the correction (the entry then is no longer
                // correctable).
                if let entry, correctionShown || ThisWasWrongDock.offered(entry) {
                    SheetGroup { ThisWasWrongDock(entry: entry) }
                        .padding(.top, 12)
                        .onAppear { correctionShown = true }
                }
                if let entry, MatrixLook.asksNextTime(entry) {
                    AskMeNextTimeDock(entry: entry)
                }
            }
            .padding(.bottom, 24)
            .onGeometryChange(for: CGFloat.self) { $0.size.height } action: { height = $0 }
        }
        .scrollBounceBehavior(.basedOnSize)
        .presentationDetents([.height(height), .large])
        .presentationDragIndicator(.visible)
        .presentationCornerRadius(32)
        .presentationBackground(Palette.paperRaised)
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("entryAutopilot")
    }

    /// "Why Autopilot denied it" reads "Why Autopilot denied" as a title.
    static func heading(_ title: String) -> String { title.hasSuffix(" it") ? String(title.dropLast(3)) : title }

    private var denied: Bool { entry?.outcome == "denied" || (entry == nil && note.suggested == .deny) }

    @ViewBuilder private var verdict: some View {
        let p = denied ? note.pDeny : note.pApprove
        HStack(spacing: 8) {
            OutcomeMark(denied ? .autoDenied : .autoApproved)
            Text(denied ? "Denied" : "Allowed")
                .font(RFont.sans(15, .semibold))
                .foregroundStyle(denied ? Palette.danger : Palette.success)
            if p > 0 {
                Text("\(SuggestionText.percent(p)) sure").font(RFont.mono(13.5)).foregroundStyle(Palette.inkMeta)
            }
        }
        .accessibilityElement(children: .combine)
    }

    private struct Reason {
        var glyph: Glyph
        var text: String
        var tint: Color? = nil
    }

    /// The facts behind it: the risk first, then its reason, the decisions it was like, the mode and profile.
    private var reasons: [Reason] {
        var out: [Reason] = []
        if let risk = note.risk, RiskLook.listed(risk) {
            let raw = untrustedName(risk.why, limit: 80)
            let why = raw.prefix(1).uppercased() + raw.dropFirst()
            out.append(Reason(glyph: .warning, text: why.isEmpty ? "Risk: \(AutopilotText.riskWord(risk.level))" : why, tint: Palette.warning))
            if risk.held { out.append(Reason(glyph: .moon, text: AutopilotText.heldLine)) }
        }
        let reason = entry.flatMap { model.reasons.lines[$0.id] } ?? untrusted(note.reason)
        if !reason.isEmpty { out.append(Reason(glyph: denied ? .close : .check, text: reason)) }
        for line in note.neighbours.prefix(3) {
            let text = untrusted(line).trimmingCharacters(in: .whitespacesAndNewlines)
            if !text.isEmpty { out.append(Reason(glyph: .chevronRight, text: text)) }
        }
        let profile = untrusted(note.profileName)
        out.append(Reason(glyph: .person, text: (["In \(ModeLook.name(note.mode))"] + (profile.isEmpty ? [] : ["profile \(profile)"])).joined(separator: " · ")))
        return out
    }
}

/// "This was wrong · Next time it asks you" (Next-Activity, 07): the last row of Autopilot's reasons on an entry it
/// (or a bypass) decided: asks what should have happened, teaches the profile and locks the kind again. What was done
/// stays done.
struct ThisWasWrongDock: View {
    var entry: ActivityEntry
    @Environment(AppModel.self) private var model
    @Environment(\.feedback) private var feedback
    @State private var asking = false
    @State private var busy = false
    @State private var thanked = false
    @State private var error: String?

    /// Only entries the core says can still be corrected.
    static func offered(_ entry: ActivityEntry) -> Bool { entry.autopilot?.correctable == true }

    var body: some View {
        let should = correctionFor(entry)
        VStack(spacing: 0) {
            if thanked {
                GlyphFact(glyph: .check, value: Correction.thanks, tint: Palette.success, glyphTint: Palette.success)
                    .accessibilityIdentifier("corrected")
            } else {
                Button {
                    asking = true
                    feedback.play(.tap)
                } label: {
                    HStack(spacing: 12) {
                        GlyphIcon(.undo, size: 20).foregroundStyle(Palette.violet).frame(width: 20)
                        VStack(alignment: .leading, spacing: 1) {
                            Text("This was wrong").textRole(.row).foregroundStyle(Palette.violet)
                            Text("Next time it asks you").textRole(.sub).foregroundStyle(Palette.inkMeta)
                        }
                        .frame(maxWidth: .infinity, alignment: .leading)
                        if busy { ProgressView() } else { RowChevron() }
                    }
                    .padding(.horizontal, 16)
                    .frame(minHeight: 56)
                    .contentShape(Rectangle())
                }
                .buttonStyle(RowButtonStyle())
                .disabled(busy)
                .accessibilityIdentifier("thisWasWrong")
            }
            if let error {
                GlyphFact(glyph: .circleAlert, value: error, tint: Palette.danger, glyphTint: Palette.danger)
            }
        }
        .alert(Correction.question(entry), isPresented: $asking) {
            Button(Correction.answer(entry)) {
                feedback.quietClose()
                Task { await correct(should) }
            }
            Button("Cancel", role: .cancel) {}
        } message: {
            Text(Correction.message)
        }
        .presentationFeedback(asking)
    }

    private func correct(_ verdict: Verdict) async {
        busy = true
        error = nil
        if let failed = await Correction.apply(model, entry: entry, shouldHave: verdict) {
            error = failed
        } else {
            thanked = true
        }
        busy = false
    }
}

/// What the matrix decided for an entry, or would have in shadow (01 §9.3), in words and a symbol (pure, unit-tested).
enum MatrixLook {
    /// "Auto · Mine · Undoable"; "Would be: Ask · Work · Reaches people" while the matrix only watches.
    static func text(_ m: MatrixView) -> String {
        let line = untrustedName(m.line, limit: 80)
        return m.shadow ? "Would be: \(line)" : line
    }

    static func symbol(_ m: MatrixView) -> String {
        if m.shadow { return "eye" }
        switch m.outcome {
        case .approve: return "bolt"
        case .ask: return m.hard ? "lock" : "person.fill.questionmark"
        case .deny: return "nosign"
        }
    }

    /// "Ask me next time" is for what Auto itself let through, under the matrix.
    static func asksNextTime(_ entry: ActivityEntry) -> Bool {
        entry.decidedBy == "autopilot" && entry.outcome != "denied" && entry.autopilot?.matrix != nil
    }
}

/// The matrix's line under an entry's facts: one quiet row with its symbol.
struct MatrixLine: View {
    var matrix: MatrixView

    var body: some View {
        HStack(spacing: 12) {
            Image(systemName: MatrixLook.symbol(matrix))
                .font(.system(size: 14, weight: .medium))
                .foregroundStyle(Palette.inkMeta)
                .frame(width: 20)
                .accessibilityHidden(true)
            Text(verbatim: MatrixLook.text(matrix))
                .font(RFont.sans(14.5))
                .foregroundStyle(Palette.ink2)
                .lineLimit(2)
                .frame(maxWidth: .infinity, alignment: .leading)
        }
        .padding(.horizontal, 16)
        .padding(.vertical, 8)
        .frame(minHeight: 44)
        .accessibilityElement(children: .combine)
        .accessibilityIdentifier("matrixLine")
    }
}

/// "Ask me next time" (01 §7) on what Auto let through: from now on that kind of thing here asks, and Auto pauses it for
/// a day. Stricter only, so nothing to confirm. Shown once the request it was is known.
struct AskMeNextTimeDock: View {
    var entry: ActivityEntry
    @Environment(AppModel.self) private var model
    @Environment(\.feedback) private var feedback
    @State private var requestId: String?
    @State private var busy = false
    @State private var done = false
    @State private var error: String?
    /// What it would have changed on this phone's last 30 days (02 §4).
    @State private var preview: String?

    var body: some View {
        Group {
            if done || requestId != nil {
                SheetGroup { content }.padding(.top, 12)
            } else {
                Color.clear.frame(height: 0)
            }
        }
        .task(id: entry.id) {
            requestId = await AskMeNextTime.requestId(entry, model: model)
            if let requestId {
                let line = await PolicyPreview.line(model.core, .askMeNextTime(requestId: requestId))
                preview = line.isEmpty ? nil : line
            }
        }
    }

    @ViewBuilder private var content: some View {
        VStack(spacing: 0) {
            if done {
                GlyphFact(glyph: .check, value: "Auto asks about these again.", tint: Palette.success, glyphTint: Palette.success)
                    .accessibilityIdentifier("askedNextTime")
            } else if let requestId {
                Button {
                    Task { await ask(requestId) }
                } label: {
                    HStack(spacing: 12) {
                        Image(systemName: "hand.raised")
                            .font(.system(size: 17, weight: .medium))
                            .foregroundStyle(Palette.violet)
                            .frame(width: 20)
                        Text("Ask me next time").textRole(.row).foregroundStyle(Palette.violet)
                            .frame(maxWidth: .infinity, alignment: .leading)
                        if busy { ProgressView() }
                    }
                    .padding(.horizontal, 16)
                    .frame(minHeight: 56)
                    .contentShape(Rectangle())
                }
                .buttonStyle(RowButtonStyle())
                .disabled(busy)
                .accessibilityIdentifier("askMeNextTime")
                if let preview { PreviewLine(line: preview) }
            }
            if let error {
                GlyphFact(glyph: .circleAlert, value: error, tint: Palette.danger, glyphTint: Palette.danger)
            }
        }
    }

    private func ask(_ requestId: String) async {
        busy = true
        error = await AskMeNextTime.apply(model, requestId: requestId)
        if error == nil { done = true }
        busy = false
    }
}

@MainActor
enum AskMeNextTime {
    /// The request an entry was: its Autopilot note says, else the approval it kept. nil for an older entry without
    /// either. The demo has no kept approvals for its seeded history, so it names one.
    static func requestId(_ entry: ActivityEntry, model: AppModel) async -> String? {
        guard MatrixLook.asksNextTime(entry) else { return nil }
        if let id = entry.autopilot?.requestId, !id.isEmpty { return id }
        if let shown = try? await model.core.activityApproval(entryId: entry.id) { return shown.requestId }
        return model.demo ? "entry-\(entry.id)" : nil
    }

    /// nil when it worked, else what to say.
    static func apply(_ model: AppModel, requestId: String) async -> String? {
        model.feedback.play(.undo)
        do {
            try await model.core.askMeNextTime(requestId: requestId)
            await model.refreshPending()
            return nil
        } catch {
            model.feedback.play(.error)
            return AutopilotModel.sentence(error.userMessage)
        }
    }
}

/// Telling Autopilot a decision was wrong: the dock on an entry's page and the swipe on its river row both do it so.
@MainActor
enum Correction {
    static func question(_ entry: ActivityEntry) -> String {
        correctionFor(entry) == .deny ? "Should this have been denied?" : "Should this have been approved?"
    }

    static func answer(_ entry: ActivityEntry) -> String {
        correctionFor(entry) == .deny ? "Deny next time" : "Approve next time"
    }

    static let message = "Autopilot learns from it. What was done stays done."
    static let thanks = "Learned. It asks about these again."

    /// Teaches the profile; nil when it worked, else what to say.
    static func apply(_ model: AppModel, entry: ActivityEntry, shouldHave verdict: Verdict) async -> String? {
        model.feedback.play(.undo)
        do {
            try await model.core.correctDecision(activityId: entry.id, shouldHave: verdict)
            await model.refreshPending()
            return nil
        } catch {
            model.feedback.play(.error)
            return error.userMessage
        }
    }
}
