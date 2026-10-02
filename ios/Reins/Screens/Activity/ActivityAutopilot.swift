import SwiftUI

// The Autopilot parts of the Activity screens (the Android app's ActivityAutopilot.kt and the mode pill): the header's
// mode pill, and an entry's "decided by Autopilot" section with "This was wrong".

/// Autopilot's modes in words, colours and symbols, as these screens show them.
enum ModeLook {
    static func name(_ mode: AutopilotMode) -> String {
        switch mode {
        case .manual: "Manual"
        case .assisted: "Assisted"
        case .auto: "Auto"
        case .bypass: "Bypass"
        case .lockdown: "Lockdown"
        }
    }

    static func tint(_ mode: AutopilotMode) -> Color {
        switch mode {
        case .manual: Palette.secondary
        case .assisted: Palette.search
        case .auto: Palette.accent
        case .bypass: Palette.danger
        case .lockdown: Palette.warning
        }
    }

    static func symbol(_ mode: AutopilotMode) -> String {
        switch mode {
        case .manual: "hand.raised.fill"
        case .assisted: "sparkles"
        case .auto: "location.north.fill"
        case .bypass: "bolt.fill"
        case .lockdown: "lock.fill"
        }
    }

    /// "14:05": minutes and seconds left.
    static func clock(until: Int64, now: Int64) -> String {
        let left = max(until - now, 0)
        return String(format: "%d:%02d", left / 60, left % 60)
    }

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
        let mode = settings.mode
        let tint = ModeLook.tint(mode)
        let strong = mode == .bypass
        Button(action: action) {
            HStack(spacing: 6) {
                if strong {
                    PulseDot()
                } else {
                    Image(systemName: ModeLook.symbol(mode)).font(.system(size: 13, weight: .semibold))
                }
                if strong, let until = settings.bypassUntil {
                    LiveClock(interval: 1) { now in
                        Text(ModeLook.clock(until: until, now: Int64(now))).font(RFont.mono(14, .semibold)).monospacedDigit()
                    }
                } else {
                    Text(ModeLook.name(mode)).font(RFont.sans(15, .medium))
                }
            }
            .lineLimit(1)
            .foregroundStyle(strong ? Color.white : (mode == .manual ? Palette.secondary : tint))
            .padding(.horizontal, 13)
            .padding(.vertical, 9)
        }
        .buttonStyle(.plain)
        .glassEffect(.regular.tint(strong ? Palette.danger : tint.opacity(mode == .manual ? 0.06 : 0.16)).interactive(), in: Capsule())
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

/// An activity entry's Autopilot part: who decided (Autopilot, a bypass, Lockdown) or what Autopilot suggested, how
/// sure it was, the decisions it was like, and "This was wrong", which teaches the profile and locks the kind again.
struct EntryAutopilotSection: View {
    var entry: ActivityEntry
    @Environment(AppModel.self) private var model
    @Environment(\.feedback) private var feedback
    @State private var asking = false
    @State private var busy = false
    @State private var thanked = false
    @State private var error: String?

    var body: some View {
        if entry.autopilot != nil || !entry.decidedBy.isEmpty { section }
    }

    private var mode: AutopilotMode {
        switch entry.decidedBy {
        case "bypass": .bypass
        case "lockdown": .lockdown
        default: entry.autopilot?.mode ?? .auto
        }
    }

    private var title: String {
        switch entry.decidedBy {
        case "autopilot": entry.outcome == "denied" ? "Denied by Autopilot" : "Approved by Autopilot"
        case "bypass": "Approved during a bypass"
        case "lockdown": "Denied by Lockdown"
        default: "Autopilot suggested: \(ModeLook.verdictWord(entry.autopilot?.suggested ?? .ask).lowercased())"
        }
    }

    private var section: some View {
        let note = entry.autopilot
        let should = correctionFor(entry)
        let tint = ModeLook.tint(mode)
        let decided = !entry.decidedBy.isEmpty
        return GroupCard(header: "Autopilot") {
            VStack(alignment: .leading, spacing: 12) {
                HStack(spacing: 14) {
                    Image(systemName: ModeLook.symbol(mode))
                        .font(.system(size: 19, weight: .semibold))
                        .foregroundStyle(decided ? Color.white : tint)
                        .frame(width: 42, height: 42)
                        .background(decided ? tint : tint.opacity(0.13), in: RoundedRectangle(cornerRadius: 13, style: .continuous))
                    VStack(alignment: .leading, spacing: 2) {
                        Text(title)
                            .font(RFont.sans(16, .semibold))
                            .foregroundStyle(Palette.text)
                            .lineLimit(2)
                            .accessibilityIdentifier("entryAutopilotTitle")
                        Text((["In \(ModeLook.name(note?.mode ?? mode))"] + [note.map { untrusted($0.profileName) }.flatMap { $0.isEmpty ? nil : "profile \($0)" }].compactMap { $0 }).joined(separator: " · "))
                            .font(RFont.sans(13))
                            .foregroundStyle(Palette.secondary)
                            .lineLimit(1)
                    }
                }
                if let note, note.pApprove > 0 || note.pDeny > 0 {
                    LikelihoodRow(label: "Approve", p: note.pApprove, color: Palette.success)
                    LikelihoodRow(label: "Deny", p: note.pDeny, color: Palette.danger)
                    LikelihoodRow(label: "Confidence", p: note.confidence, color: Palette.accent)
                }
                if let reason = note.map({ untrusted($0.reason) }), !reason.isEmpty {
                    Text(reason).font(RFont.sans(14.5)).foregroundStyle(Palette.text)
                }
                if let neighbours = note?.neighbours, !neighbours.isEmpty {
                    VStack(alignment: .leading, spacing: 6) {
                        Caption("Like these decisions of yours")
                        ForEach(Array(neighbours.enumerated()), id: \.offset) { _, line in
                            HStack(alignment: .firstTextBaseline, spacing: 6) {
                                Image(systemName: "chevron.right").font(.system(size: 10, weight: .semibold)).foregroundStyle(Palette.tertiary)
                                Text(untrusted(line)).font(RFont.sans(13.5)).foregroundStyle(Palette.secondary).lineLimit(2)
                            }
                        }
                    }
                }
                if thanked {
                    Banner("Thanks. Autopilot learned from this and asks you about requests like it again.")
                        .accessibilityIdentifier("corrected")
                } else if note?.correctable == true {
                    Button {
                        asking = true
                        feedback.play(.tap)
                    } label: {
                        ZStack {
                            Label("This was wrong", systemImage: "flag").opacity(busy ? 0 : 1)
                            if busy { ProgressView() }
                        }
                    }
                    .buttonStyle(CapsuleButtonStyle(kind: .secondary, height: 46))
                    .disabled(busy)
                    .accessibilityIdentifier("thisWasWrong")
                }
                if let error { Banner(error, kind: .error) }
            }
            .padding(16)
            .accessibilityIdentifier("entryAutopilot")
        }
        .alert(should == .deny ? "Should this have been denied?" : "Should this have been approved?", isPresented: $asking) {
            Button(should == .deny ? "Deny next time" : "Approve next time") { Task { await correct(should) } }
            Button("Cancel", role: .cancel) {}
        } message: {
            Text((should == .deny ? "Autopilot remembers it as a denial" : "Autopilot remembers it as an approval")
                + ", more strongly than an ordinary answer, and asks you about this kind of request again until it has learned more. What was done stays done.")
        }
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
