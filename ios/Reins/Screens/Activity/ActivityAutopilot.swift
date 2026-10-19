import SwiftUI

// The Autopilot parts of the Activity screens (the Android app's ActivityAutopilot.kt): the modes in words, an entry's
// outcome pill, "Why Autopilot allowed it" and the "This was wrong" dock. The header's mode chip is ActivityModeMenu.

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
    static func text(_ entry: ActivityEntry) -> String {
        switch entry.outcome {
        case "started": return "Awaiting outcome"
        case "unknown": return "Outcome unknown · do not repeat"
        case "succeeded_withheld": return "Completed; response withheld · do not repeat"
        case "error": return "Failed"
        default: break
        }
        let denied = entry.outcome == "denied"
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
        case .grant: "key.horizontal"
        case .denied: "xmark"
        case .failed, .started, .unknown, .withheld: "exclamationmark.triangle"
        case .approved: "checkmark"
        }
    }

    static func tint(_ entry: ActivityEntry) -> Color {
        switch EntryText.mark(entry) {
        case .auto, .grant: Palette.violet
        case .overnight: Palette.violet
        case .bypass, .denied: Palette.danger
        case .lockdown, .failed, .started, .unknown, .withheld: Palette.warning
        case .approved: Palette.success
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
                       risk: entry.autopilot?.risk)
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

/// The history row's risk: the level in a few words in its colour, the moon when Overnight held it.
struct RiskLine: View {
    var risk: RiskView

    var body: some View {
        let tint = RiskLook.tint(risk.level)
        HStack(spacing: 4) {
            Image(systemName: RiskLook.symbol(risk.level)).font(.system(size: 11, weight: .semibold))
            Text("Risk: \(AutopilotText.riskWord(risk.level))").font(RFont.sans(12.5, .medium))
            if risk.held {
                Image(systemName: AutopilotText.overnightSymbol)
                    .font(.system(size: 11, weight: .semibold))
                    .foregroundStyle(Palette.violet)
                    .accessibilityLabel(AutopilotText.heldLine)
            }
        }
        .foregroundStyle(tint)
        .lineLimit(1)
        .accessibilityElement(children: .combine)
        .accessibilityIdentifier("entryRisk")
    }
}

/// "Why Autopilot allowed it ›": opens in place to how sure it was, its reason, the decisions of yours it was like and
/// the profile. Only for entries Autopilot looked at.
struct AutopilotWhyRow: View {
    var note: AutopilotNote
    var title: String
    /// The entry it is about, for the reason line worked out on the phone.
    var entry: ActivityEntry? = nil
    @State private var open = false
    @Environment(\.feedback) private var feedback
    @Environment(AppModel.self) private var model

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            Button {
                withAnimation(.spring(duration: 0.35, bounce: 0.1)) { open.toggle() }
                feedback.play(.expand(open))
            } label: {
                HStack(spacing: 8) {
                    Text(title).font(RFont.sans(16, .medium)).foregroundStyle(Palette.ink).lineLimit(1)
                    Spacer(minLength: 8)
                    Image(systemName: "chevron.right")
                        .font(.system(size: 13, weight: .semibold))
                        .foregroundStyle(Palette.inkMeta)
                        .rotationEffect(.degrees(open ? 90 : 0))
                }
                .padding(.horizontal, 16)
                .frame(minHeight: 52)
                .contentShape(Rectangle())
            }
            .buttonStyle(RowButtonStyle())
            .accessibilityIdentifier("entryAutopilotTitle")
            .accessibilityValue(open ? "Open" : "Closed")
            if open {
                detail.transition(.opacity)
            }
        }
        .task(id: entry?.id) {
            if let entry { await model.reasons.explain(entry, model: model) }
        }
        .accessibilityContainer("entryAutopilot")
    }

    private var detail: some View {
        VStack(alignment: .leading, spacing: 10) {
            if note.pApprove > 0 || note.pDeny > 0 {
                LikelihoodRow(label: "Approve", p: note.pApprove, color: Palette.success)
                LikelihoodRow(label: "Deny", p: note.pDeny, color: Palette.danger)
                LikelihoodRow(label: "Confidence", p: note.confidence, color: Palette.violet)
            }
            // The decisions of yours it was like, counted here ("Like 11 pushes to feature branches of acme/web you
            // approved."), else the core's own reason.
            let reason = entry.flatMap { model.reasons.lines[$0.id] } ?? untrusted(note.reason)
            if !reason.isEmpty {
                Text(reason).textRole(.body).foregroundStyle(Palette.ink2).accessibilityIdentifier("autopilotReason")
            }
            if !note.neighbours.isEmpty {
                VStack(alignment: .leading, spacing: 6) {
                    Caption("Like these decisions of yours")
                    ForEach(Array(note.neighbours.enumerated()), id: \.offset) { _, line in
                        HStack(alignment: .firstTextBaseline, spacing: 6) {
                            Image(systemName: "chevron.right").font(.system(size: 10, weight: .semibold)).foregroundStyle(Palette.inkMeta)
                            Text(untrusted(line)).font(RFont.sans(13.5)).foregroundStyle(Palette.ink2).lineLimit(2)
                        }
                    }
                }
            }
            Text((["In \(ModeLook.name(note.mode))"] + [untrusted(note.profileName)].filter { !$0.isEmpty }.map { "profile \($0)" }).joined(separator: " · "))
                .font(RFont.sans(12.5))
                .foregroundStyle(Palette.inkMeta)
        }
        .padding(.horizontal, 16)
        .padding(.bottom, 14)
    }
}

/// "This was wrong", pinned at the bottom of an entry Autopilot (or a bypass) decided: asks what should have
/// happened, teaches the profile and locks the kind again. What was done stays done.
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
        VStack(spacing: 10) {
            if let error { Banner(error, kind: .error) }
            if thanked {
                Banner(Correction.thanks)
                    .accessibilityIdentifier("corrected")
            } else {
                // A normal neutral button like Deny: only its undo mark is in Autopilot's violet, since it teaches Autopilot.
                Button {
                    asking = true
                    feedback.play(.tap)
                } label: {
                    ZStack {
                        Label {
                            Text("This was wrong")
                        } icon: {
                            GlyphIcon(.undo, size: 18).foregroundStyle(Palette.violet)
                        }
                        .opacity(busy ? 0 : 1)
                        if busy { ProgressView() }
                    }
                }
                .buttonStyle(DockButtonStyle(kind: .neutral))
                .disabled(busy)
                .accessibilityIdentifier("thisWasWrong")
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
