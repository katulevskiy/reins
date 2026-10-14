import SwiftUI

// The Autopilot parts of the Activity screens (the Android app's ActivityAutopilot.kt and the mode pill): the header's
// mode pill, an entry's outcome pill, "Why Autopilot allowed it" and the "This was wrong" dock.

/// Autopilot's modes in words, colours and symbols, as these screens show them.
enum ModeLook {
    static func name(_ mode: AutopilotMode) -> String {
        switch mode {
        case .manual: "Manual"
        case .auto: "Auto"
        case .bypass: "Bypass"
        case .lockdown: "Lockdown"
        }
    }

    static func tint(_ mode: AutopilotMode) -> Color {
        switch mode {
        case .manual: Palette.search
        case .auto: Palette.accent
        case .bypass: Palette.danger
        case .lockdown: Palette.warning
        }
    }

    static func symbol(_ mode: AutopilotMode) -> String {
        switch mode {
        case .manual: "person.fill.checkmark"
        case .auto: "bolt"
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

/// The header's Autopilot pill: the global mode at a glance. Red with a live countdown while a bypass runs, amber in
/// Lockdown, quiet otherwise.
struct ActivityModePill: View {
    var settings: AutopilotSettings
    var action: () -> Void

    var body: some View {
        Button(action: action) { ModePillLabel(settings: settings) }
            .buttonStyle(.plain)
            .modePillGlass(settings)
    }
}

/// What the mode pill shows: the mode's symbol and name, or a pulse and the time left while a bypass runs (a pulse
/// and "Bypass" for one without an end).
struct ModePillLabel: View {
    var settings: AutopilotSettings

    var body: some View {
        let mode = settings.mode
        let tint = ModeLook.tint(mode)
        let strong = mode == .bypass
        HStack(spacing: 6) {
            if strong {
                PulseDot()
            } else {
                Image(systemName: ModeLook.symbol(mode)).font(.system(size: 13, weight: .semibold))
            }
            // A bypass without an end has nothing to count down: it reads as the mode, as on Android.
            if strong, let until = settings.bypassUntil, !AutopilotText.isUntilOff(until) {
                LiveClock(interval: 1) { now in
                    Text(ModeLook.clock(until: until, now: Int64(now))).font(RFont.mono(14, .semibold)).monospacedDigit()
                }
            } else {
                Text(ModeLook.name(mode)).font(RFont.sans(15, .medium))
            }
        }
        .lineLimit(1)
        .foregroundStyle(strong ? Color.white : tint)
        .padding(.horizontal, 13)
        .frame(height: HeaderControl.height)
    }
}

extension View {
    /// The mode pill's glass: red during a bypass, the mode's tint otherwise, with its label and identifier.
    func modePillGlass(_ settings: AutopilotSettings) -> some View {
        let mode = settings.mode
        let tint = ModeLook.tint(mode)
        let strong = mode == .bypass
        return glassEffect(.regular.tint(strong ? Palette.danger : tint.opacity(0.16)).interactive(), in: Capsule())
            .accessibilityLabel("Autopilot: \(ModeLook.name(mode))")
            .accessibilityIdentifier("modePill")
    }
}

/// A dot that breathes, for something running now. Still when clocks are frozen (screenshots).
private struct PulseDot: View {
    @State private var dim = false

    var body: some View {
        Circle()
            .fill(Color.white)
            .frame(width: 8, height: 8)
            .opacity(dim ? 0.35 : 1)
            .onAppear {
                guard Timers.live else { return }
                withAnimation(.linear(duration: 0.9).repeatForever(autoreverses: true)) { dim = true }
            }
    }
}

/// What should have happened instead of what did: a denial was a mistaken approval's correction, and the reverse.
func correctionFor(_ entry: ActivityEntry) -> Verdict { entry.outcome == "denied" ? .approve : .deny }

/// The outcome pill under an entry's sentence ("Allowed by Autopilot · 96%"), in words, symbol and colour (pure,
/// unit-tested).
enum EntryOutcome {
    static func text(_ entry: ActivityEntry) -> String {
        let denied = entry.outcome == "denied"
        switch entry.decidedBy {
        case "autopilot":
            let p = entry.autopilot.map { denied ? $0.pDeny : $0.pApprove }
            let base = denied ? "Denied by Autopilot" : "Allowed by Autopilot"
            return p.map { $0 > 0 ? "\(base) · \(SuggestionText.percent($0))" : base } ?? base
        case "bypass": return "Allowed during a bypass"
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
        case .bypass: "bolt.fill"
        case .lockdown: "lock.fill"
        case .grant: "key.horizontal"
        case .denied: "xmark"
        case .failed: "exclamationmark.triangle"
        case .approved: "checkmark"
        }
    }

    static func tint(_ entry: ActivityEntry) -> Color {
        switch EntryText.mark(entry) {
        case .auto, .grant: Palette.accent
        case .bypass, .denied: Palette.danger
        case .lockdown, .failed: Palette.warning
        case .approved: Palette.success
        }
    }

    /// The row that opens Autopilot's reasons: "Why Autopilot allowed it", "Why Autopilot denied it", "What Autopilot
    /// suggested".
    static func whyTitle(_ entry: ActivityEntry) -> String {
        switch entry.decidedBy {
        case "autopilot": entry.outcome == "denied" ? "Why Autopilot denied it" : "Why Autopilot allowed it"
        default: "What Autopilot suggested"
        }
    }
}

/// "Why Autopilot allowed it ›": opens in place to how sure it was, its reason, the decisions of yours it was like and
/// the profile. Only for entries Autopilot looked at.
struct AutopilotWhyRow: View {
    var note: AutopilotNote
    var title: String
    @State private var open = false
    @Environment(\.feedback) private var feedback

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            Button {
                withAnimation(.spring(duration: 0.35, bounce: 0.1)) { open.toggle() }
                feedback.play(.expand(open))
            } label: {
                HStack(spacing: 8) {
                    Text(title).font(RFont.sans(16, .medium)).foregroundStyle(Palette.text).lineLimit(1)
                    Spacer(minLength: 8)
                    Image(systemName: "chevron.right")
                        .font(.system(size: 13, weight: .semibold))
                        .foregroundStyle(Palette.tertiary)
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
        .accessibilityContainer("entryAutopilot")
    }

    private var detail: some View {
        VStack(alignment: .leading, spacing: 10) {
            if note.pApprove > 0 || note.pDeny > 0 {
                LikelihoodRow(label: "Approve", p: note.pApprove, color: Palette.success)
                LikelihoodRow(label: "Deny", p: note.pDeny, color: Palette.danger)
                LikelihoodRow(label: "Confidence", p: note.confidence, color: Palette.accent)
            }
            let reason = untrusted(note.reason)
            if !reason.isEmpty {
                Text(reason).font(RFont.sans(14.5)).foregroundStyle(Palette.text)
            }
            if !note.neighbours.isEmpty {
                VStack(alignment: .leading, spacing: 6) {
                    Caption("Like these decisions of yours")
                    ForEach(Array(note.neighbours.enumerated()), id: \.offset) { _, line in
                        HStack(alignment: .firstTextBaseline, spacing: 6) {
                            Image(systemName: "chevron.right").font(.system(size: 10, weight: .semibold)).foregroundStyle(Palette.tertiary)
                            Text(untrusted(line)).font(RFont.sans(13.5)).foregroundStyle(Palette.secondary).lineLimit(2)
                        }
                    }
                }
            }
            Text((["In \(ModeLook.name(note.mode))"] + [untrusted(note.profileName)].filter { !$0.isEmpty }.map { "profile \($0)" }).joined(separator: " · "))
                .font(RFont.sans(12.5))
                .foregroundStyle(Palette.tertiary)
        }
        .padding(.horizontal, 16)
        .padding(.bottom, 14)
    }
}

/// What should have happened instead of what did: a denial was a mistaken approval's correction, and the reverse.
func correctionFor(_ entry: ActivityEntry) -> Verdict { entry.outcome == "denied" ? .approve : .deny }

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
                Banner("Learned. It asks about these again.")
                    .accessibilityIdentifier("corrected")
            } else {
                Button {
                    asking = true
                    feedback.play(.tap)
                } label: {
                    ZStack {
                        Label("This was wrong", systemImage: "arrow.uturn.backward").opacity(busy ? 0 : 1)
                        if busy { ProgressView() }
                    }
                }
                .buttonStyle(DockButtonStyle(kind: .warning))
                .disabled(busy)
                .accessibilityIdentifier("thisWasWrong")
            }
        }
        .alert(should == .deny ? "Should this have been denied?" : "Should this have been approved?", isPresented: $asking) {
            Button(should == .deny ? "Deny next time" : "Approve next time") {
                feedback.quietClose()
                Task { await correct(should) }
            }
            Button("Cancel", role: .cancel) {}
        } message: {
            Text("Autopilot learns from it. What was done stays done.")
        }
        .presentationFeedback(asking)
    }

    private func correct(_ verdict: Verdict) async {
        busy = true
        error = nil
        feedback.play(.undo)
        do {
            try await model.core.correctDecision(activityId: entry.id, shouldHave: verdict)
            await model.refreshPending()
            thanked = true
        } catch {
            feedback.play(.error)
            self.error = error.userMessage
        }
        busy = false
    }
}
