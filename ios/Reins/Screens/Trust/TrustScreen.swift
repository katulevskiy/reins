import SwiftUI

/// Trust: everything that runs without asking, in one place (iOS-Trust.png). Autopilot's mode up top (Manual and Auto,
/// with Overnight, Bypass and Lock down at hand), this week's numbers, the grants that are live, then the model and
/// "Try it", the profiles, and the starting rule for new AIs. It is the Autopilot page and the Grants page put together:
/// their views, state and sheets moved here as they were.
struct TrustScreen: View {
    @Environment(AppModel.self) private var model
    @Environment(\.feedback) private var feedback
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    // Autopilot's.
    @State private var ap = AutopilotModel()
    @State private var bypassAsk = false
    @State private var overnightAsk = false
    @State private var mobileDataAsk = false
    @State private var modelOpen = false
    @State private var newProfile = false
    // The grants'.
    @State private var revoking: GrantView?
    @State private var grantError: String?

    private var running: [GrantView] { model.grants.filter(\.active).sorted { $0.createdAt > $1.createdAt } }
    private var endedCount: Int { model.grants.filter { !$0.active }.count }

    /// A bypass, an Overnight or a lockdown running: the field on top.
    private var runningMode: TrustRunning? { ap.settings.flatMap(TrustRunning.of) }

    var body: some View {
        List {
            PageHeader(title: "Trust") {
                RoundPlusButton(label: "New grant") { model.show(.newGrant(token: 0)) }
                    .accessibilityIdentifier("newGrant")
            }
            .padding(.top, 8)
            .plainRow(EdgeInsets(top: 0, leading: 20, bottom: 0, trailing: 16))

            if let s = ap.settings {
                autopilot(s)
            } else {
                ProgressView().padding(.vertical, 40).frame(maxWidth: .infinity).plainRow(EdgeInsets())
            }

            grants

            if let s = ap.settings {
                modelAndProfiles(s)
            }

            // The starting rule for new AIs (the card brings its own side margin).
            StartingRuleChooser()
                .padding(.top, 6)
                .plainRow(EdgeInsets(top: 6, leading: 0, bottom: 6, trailing: 0))
        }
        .listStyle(.plain)
        .scrollContentBackground(.hidden)
        .contentMargins(.bottom, 24, for: .scrollContent)
        .safeAreaInset(edge: .top, spacing: 0) {
            if let running = runningMode, let s = ap.settings {
                TrustRunningField(
                    running: running, settings: s, top: 0,
                    onStopBypass: { Task { await ap.stopBypass() } },
                    onStopOvernight: { Task { await ap.stopOvernight() } },
                    onEndLockdown: { Task { await ap.endLockdown() } }
                )
                .transition(.move(edge: .top).combined(with: .opacity))
            }
        }
        .animation(Motion.of(.smooth(duration: 0.32), reduce: reduceMotion), value: runningMode)
        .pageBackground()
        .navigationTitle("Trust")
        .rootNavigationBar()
        // White status bar text on a running mode's field.
        .toolbarColorScheme(runningMode == nil ? nil : .dark, for: .navigationBar)
        .refreshable {
            feedback.play(.refresh)
            await model.refreshPending()
            await ap.refresh()
            await ap.loadStats()
        }
        .task {
            ap.bind(model)
            await ap.refresh()
            await ap.loadStats()
            #if DEBUG
            // `-openSheet bypass|overnight|model` (screenshots): open that sheet at launch.
            let args = ProcessInfo.processInfo.arguments
            if let i = args.firstIndex(of: "-openSheet"), i + 1 < args.count {
                switch args[i + 1] {
                case "bypass": bypassAsk = true
                case "overnight": overnightAsk = true
                case "model": modelOpen = true
                default: break
                }
            }
            #endif
        }
        .task(id: [ap.settings?.bypassUntil, ap.settings?.overnightUntil]) {
            // The core ends a bypass or an Overnight when it is read after its time; read it then, so the screen moves
            // on by itself. One without an end waits for the user.
            let ends = [ap.settings?.bypassUntil, ap.settings?.overnightUntil].compactMap { $0 }.filter { !AutopilotText.isUntilOff($0) }
            guard let until = ends.min() else { return }
            let left = Double(until) - Date().timeIntervalSince1970
            try? await Task.sleep(for: .seconds(max(left, 0) + 1))
            if !Task.isCancelled { await model.refreshAutopilot() }
        }
        .sheet(isPresented: $bypassAsk) {
            BypassSheet(who: nil) { length in
                feedback.quietClose()
                Task { await ap.startBypass(minutes: length.minutes) }
            }
            .environment(\.feedback, feedback)
        }
        .presentationFeedback(bypassAsk)
        .sheet(isPresented: $overnightAsk) {
            OvernightSheet(who: nil, ap: ap) { length in
                feedback.quietClose()
                Task { await ap.startOvernight(minutes: length.minutes) }
            }
            .environment(\.feedback, feedback)
        }
        .presentationFeedback(overnightAsk)
        .sheet(isPresented: $modelOpen) {
            ModelSheet(ap: ap)
                .environment(\.feedback, feedback)
        }
        .presentationFeedback(modelOpen)
        .sheet(isPresented: $newProfile) {
            ProfileEditorSheet(title: "New profile", initialName: "", initialIcon: AutopilotText.icons[2], confirmLabel: "Create") { name, icon in
                feedback.quietClose()
                Task {
                    if let id = await ap.createProfile(name: name, icon: icon) { model.openFromAutopilot(.autopilotProfile(id)) }
                }
            }
        }
        .presentationFeedback(newProfile)
        .mobileDataPrompt(isPresented: $mobileDataAsk, ap: ap)
        .confirmationDialog(
            "End this grant now?",
            isPresented: Binding(get: { revoking != nil }, set: { if !$0 { revoking = nil } }),
            titleVisibility: .visible,
            presenting: revoking
        ) { grant in
            Button("End now", role: .destructive) {
                grantError = nil
                feedback.quietClose()
                Task { grantError = await model.revokeGrant(grant.id) }
            }
        } message: { grant in
            Text(untrusted(grant.summary) + "\n\nYou can resume it later from Ended.")
        }
        .presentationFeedback(revoking != nil)
    }

    // MARK: Autopilot

    @ViewBuilder
    private func autopilot(_ s: AutopilotSettings) -> some View {
        Text(TrustText.decided(model.activity, now: Timers.nowSeconds()))
            .textRole(.mood)
            .foregroundStyle(Palette.ink2)
            .accessibilityIdentifier("trustMood")
            .plainRow(EdgeInsets(top: 0, leading: 24, bottom: 4, trailing: 24))
        if let running = runningMode {
            runningList(running, s)
        }
        VStack(alignment: .leading, spacing: 8) {
            // "Mode" with its (?) right beside the word; what the modes do opens under it. While a bypass or an Overnight
            // runs, the field above has the mode.
            if runningMode == nil || runningMode == .lockdown {
                HelpHeader(title: "Mode", help: AutopilotText.modeHelp, id: "mode")
                    .padding(.horizontal, 12)
                    .accessibilityElement(children: .contain)
                    .accessibilityIdentifier("modeHeader")
            }
            ModeHero(
                settings: s,
                modelReady: ap.modelReady,
                onPick: { mode in if mode != s.mode { Task { await ap.setMode(mode) } } },
                onBypass: { bypassAsk = true },
                onOvernight: { overnightAsk = true },
                onLockdown: { Task { await ap.setMode(.lockdown) } }
            )
        }
        .plainRow(EdgeInsets(top: 6, leading: 16, bottom: 6, trailing: 16))

        if ap.error != nil || ap.notice != nil {
            VStack(spacing: 8) {
                if let error = ap.error { IntegrationBanner(text: error, kind: .error).accessibilityIdentifier("autopilotError") }
                if let notice = ap.notice { IntegrationBanner(text: notice).accessibilityIdentifier("autopilotNotice") }
            }
            .plainRow(EdgeInsets(top: 6, leading: 16, bottom: 6, trailing: 16))
        }

        Group {
            if ap.modelReady {
                WeekStats(stats: ap.stats).padding(.horizontal, 8)
            } else if let m = ap.model {
                ModelOffer(ap: ap, model: m, onDownload: download)
            }
        }
        .plainRow(EdgeInsets(top: 10, leading: 16, bottom: 6, trailing: 16))
    }

    @ViewBuilder
    private func modelAndProfiles(_ s: AutopilotSettings) -> some View {
        AutopilotGroup(header: nil) {
            ModelRow(ap: ap, onUpdate: download) { modelOpen = true }
            RowDivider(inset: 60)
            AutopilotLinkRow(title: "Try it", symbol: "flask.fill", tint: Palette.send, identifier: "openTryIt") {
                if !ap.modelReady {
                    Text("after download").font(RFont.sans(15)).foregroundStyle(Palette.tertiary).lineLimit(1)
                }
            } action: {
                model.openFromAutopilot(.tryIt(nil))
            }
        }
        .plainRow(EdgeInsets(top: 18, leading: 16, bottom: 6, trailing: 16))

        VStack(alignment: .leading, spacing: 8) {
            HStack {
                SectionHeader("Profiles")
                Button {
                    feedback.play(.tap)
                    newProfile = true
                } label: {
                    Label("New", systemImage: "plus")
                        .labelStyle(.titleAndIcon)
                        .font(RFont.sans(14.5, .medium))
                        .foregroundStyle(Palette.accent)
                        .frame(minHeight: 30)
                        .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .accessibilityIdentifier("newProfile")
            }
            .padding(.horizontal, 12)
            AutopilotGroup(header: nil) {
                ForEach(Array(ap.profiles.enumerated()), id: \.element.id) { i, profile in
                    if i > 0 { RowDivider(inset: 64) }
                    ProfileRow(profile: profile, line: AutopilotText.profileLine(profile, ais: ais(profile, s))) {
                        model.openFromAutopilot(.autopilotProfile(profile.id))
                    }
                }
            }
        }
        .plainRow(EdgeInsets(top: 18, leading: 16, bottom: 6, trailing: 16))
    }

    private func ais(_ p: ProfileView, _ s: AutopilotSettings) -> [String] {
        AutopilotText.profileAis(p, settings: s, connections: model.connections.map { ($0.id, $0.label) })
    }

    private func download() {
        if ap.needsMobileDataConsent() { mobileDataAsk = true } else { ap.download() }
    }

    // MARK: Grants

    /// "ACTIVE GRANTS · 3 ……… Ended ›", then each running grant as a hairline row with its time left. Swiping one ends it
    /// (it can be resumed later from Ended).
    @ViewBuilder
    private var grants: some View {
        let running = running
        let endedCount = endedCount
        PageSectionHeader(running.isEmpty ? "Active grants" : "Active grants · \(running.count)") {
            if endedCount > 0 {
                Button {
                    model.show(.expiredGrants)
                } label: {
                    HStack(spacing: 3) {
                        Text("Ended")
                        Text("\(endedCount)").foregroundStyle(Palette.inkMeta)
                        GlyphIcon(.chevronRight, size: 13).foregroundStyle(Palette.inkFaint)
                    }
                    .font(RFont.sans(13.5, .medium))
                    .foregroundStyle(Palette.ink2)
                    .frame(minHeight: 30)
                    .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .feedbackTap(.tap, feedback)
                .accessibilityElement(children: .combine)
                .accessibilityLabel("Ended, \(endedCount)")
                .accessibilityIdentifier("expiredHeader")
            }
        }
        .padding(.top, 14)
        .plainRow(EdgeInsets(top: 0, leading: 2, bottom: 0, trailing: 8))

        if let grantError {
            FormBanner(text: grantError)
                .accessibilityIdentifier("resumeError")
                .plainRow(EdgeInsets(top: 6, leading: 16, bottom: 6, trailing: 16))
        }

        if running.isEmpty {
            Text("Nothing runs without asking.")
                .textRole(.body)
                .foregroundStyle(Palette.inkMeta)
                .accessibilityIdentifier("noGrants")
                .plainRow(EdgeInsets(top: 4, leading: 24, bottom: 8, trailing: 24))
        }

        ForEach(running, id: \.id) { grant in
            Button {
                model.show(.grantDetail(grant.id))
            } label: {
                LiveGrantCard(grant: grant, row: true)
            }
            .buttonStyle(.plain)
            .feedbackTap(.tap, feedback)
            .accessibilityIdentifier("grant:\(grant.id)")
            .swipeActions(edge: .trailing, allowsFullSwipe: false) {
                Button { tapped { revoking = grant } } label: { Label("End now", systemImage: "nosign") }
                    .tint(Palette.danger)
            }
            .contextMenu {
                Button { tapped { model.show(.grantDetail(grant.id)) } } label: { Label("Open", systemImage: "key.horizontal") }
                Button(role: .destructive) { tapped { revoking = grant } } label: { Label("End now", systemImage: "nosign") }
            }
            .plainRow(EdgeInsets(top: 0, leading: 24, bottom: 0, trailing: 24))
        }
    }

    /// What the running mode decided ("During this bypass"), and what Overnight does with what is too risky.
    @ViewBuilder
    private func runningList(_ running: TrustRunning, _ s: AutopilotSettings) -> some View {
        if case .overnight = running {
            WhenRiskyPicker(selected: s.overnightWhenRisky) { risky in Task { await ap.setOvernightWhenRisky(risky) } }
                .plainRow(EdgeInsets(top: 16, leading: 16, bottom: 4, trailing: 16))
        }
        let entries = running.entries(model.activity, settings: s)
        if !entries.isEmpty {
            PageSectionHeader(running.listTitle) { EmptyView() }
                .padding(.top, 14)
                .plainRow(EdgeInsets(top: 0, leading: 2, bottom: 0, trailing: 8))
            ForEach(entries, id: \.id) { entry in
                Button {
                    model.show(.activityDetail(entry.id), in: .activity)
                } label: {
                    HistoryRow(entry: entry)
                }
                .buttonStyle(.plain)
                .plainRow(EdgeInsets())
            }
        }
    }

    /// A menu or swipe item: its action, then the default tap (the page or dialog it opens has the sound).
    private func tapped(_ action: () -> Void) {
        action()
        feedback.defaultTap()
    }
}

/// What Trust says in words.
enum TrustText {
    /// The mood line under the title: what Autopilot (or Overnight) decided today ("Autopilot decided 9 things today.").
    static func decided(_ entries: [ActivityEntry], now: Int64) -> String {
        let calendar = Calendar.current
        let today = calendar.startOfDay(for: Date(timeIntervalSince1970: TimeInterval(now)))
        let n = entries.filter {
            ["autopilot", "overnight"].contains($0.decidedBy) && calendar.startOfDay(for: Date(timeIntervalSince1970: TimeInterval($0.at))) == today
        }.count
        switch n {
        case 0: return "Autopilot decided nothing yet today."
        case 1: return "Autopilot decided 1 thing today."
        default: return "Autopilot decided \(n) things today."
        }
    }

    /// The mood line under the title: what Autopilot did this week, when its model runs ("Autopilot scored 87 requests
    /// this week."); nil without numbers.
    static func mood(_ stats: AutopilotStats?) -> String? {
        guard let stats else { return nil }
        switch stats.scored {
        case 0: return "Autopilot has not scored anything this week."
        case 1: return "Autopilot scored one request this week."
        default: return "Autopilot scored \(stats.scored) requests this week."
        }
    }
}
