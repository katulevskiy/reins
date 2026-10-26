import SwiftUI

/// Trust (the Trust, Mode picker and Running boards): the mode now, the grants that run without asking, Permissions,
/// and Autopilot's profiles, "Try it" and model. Calm (Manual or Auto) it is a standard page under its title, with the
/// Manual/Auto switch and one "For a while" row. While Overnight, Lockdown or Bypass runs, Trust becomes that mode's
/// field: its name and count in the island's ears, a big clock and Stop now, collapsing in two steps as the page
/// scrolls.
struct TrustScreen: View {
    @Environment(AppModel.self) private var model
    @Environment(\.feedback) private var feedback
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @Environment(\.dynamicTypeSize) private var typeSize
    // Autopilot's.
    @State private var ap = AutopilotModel()
    @State private var bypassAsk = false
    @State private var mobileDataAsk = false
    @State private var modelOpen = false
    @State private var newProfile = false
    // The grants'.
    @State private var revoking: GrantView?
    @State private var grantError: String?
    // A running mode's field.
    @State private var fold = TrustFold()
    @State private var safeTop: CGFloat = 59
    /// The running mode's list is open; folded each time Trust opens.
    @State private var runningListOpen = false

    private var running: [GrantView] { model.grants.filter(\.active).sorted { $0.createdAt > $1.createdAt } }
    private var endedCount: Int { model.grants.filter { !$0.active }.count }

    /// Overnight, Lockdown or Bypass running: Trust is its field.
    private var runningMode: TrustRunning? { ap.settings.flatMap(TrustRunning.of) }

    private var placement: EarsPlacement { EarsPlacement(safeTop: safeTop, accessibility: typeSize.isAccessibilitySize) }

    var body: some View {
        let mode = runningMode
        let placement = placement
        ZStack(alignment: .top) {
            list(mode, placement)
            if let mode, let s = ap.settings {
                RunningOverlay(
                    running: mode, count: mode.entries(model.activity, settings: s).count, placement: placement, fold: fold,
                    onStop: { stop(mode) }
                )
                .ignoresSafeArea(edges: .top)
                .transition(.opacity)
            }
        }
        .onGeometryChange(for: CGFloat.self) { $0.safeAreaInsets.top } action: { top in
            if top > 0 { safeTop = top }
        }
        .onChange(of: placement, initial: true) { _, p in
            fold.shortHeight = p.shortHeight
            fold.miniHeight = p.miniHeight
        }
        .animation(Motion.of(.smooth(duration: 0.32), reduce: reduceMotion), value: mode)
        // Trust opens with the running list folded (a tab keeps its page, so fold it when the tab is chosen).
        .onChange(of: model.section) { _, section in if section == .trust { runningListOpen = false } }
        .onChange(of: mode) {
            runningListOpen = false
            fold.reset()
        }
        .pageBackground()
        .navigationTitle("Trust")
        .rootNavigationBar()
        // The ears take the status bar's place beside the island; under a running field the status bar is white.
        .statusBarHidden(mode != nil && placement.hidesStatusBar && model.section == .trust)
        .toolbarColorScheme(mode == nil ? nil : .dark, for: .navigationBar)
        .refreshable {
            feedback.play(.refresh)
            await model.refreshPending()
            await ap.refresh()
        }
        .task {
            ap.bind(model)
            await ap.refresh()
            #if DEBUG
            // `-openSheet bypass|model` (screenshots): open that sheet at launch.
            let args = ProcessInfo.processInfo.arguments
            if let i = args.firstIndex(of: "-openSheet"), i + 1 < args.count {
                switch args[i + 1] {
                case "bypass": bypassAsk = true
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

    private func list(_ mode: TrustRunning?, _ placement: EarsPlacement) -> some View {
        List {
            if let mode, let s = ap.settings {
                RunningFieldFull(running: mode, placement: placement, current: fold.stage == .full) { stop(mode) }
                    .onGeometryChange(for: CGFloat.self) { $0.size.height } action: { fold.fieldBottom = $0 }
                    .plainRow(EdgeInsets())
                    .id("field")
                    #if DEBUG
                    .background(DebugScroll())
                    #endif
                runningMiddle(mode, s)
                    .id("middle")
            } else {
                PageHeader(title: "Trust") { EmptyView() }
                    .padding(.top, placement.safeTop + 4)
                    .padding(.bottom, 24)
                    .plainRow(EdgeInsets(top: 0, leading: 20, bottom: 0, trailing: 16))
                if let s = ap.settings {
                    TrustModePicker(
                        settings: s,
                        autoReady: ap.modelReady,
                        onPick: { picked in if picked != s.mode { Task { await ap.setMode(picked) } } },
                        onOvernight: { Task { await ap.startOvernight(minutes: TimedMode.overnight.initial) } },
                        onLockdown: { Task { await ap.setMode(.lockdown) } },
                        onBypass: { bypassAsk = true }
                    )
                    .accessibilityElement(children: .contain)
                    .accessibilityIdentifier("modeHero")
                    .plainRow(EdgeInsets())
                    if !ap.modelReady, let m = ap.model {
                        ModelOffer(ap: ap, model: m, onDownload: download)
                            .padding(.top, 12)
                            .plainRow(EdgeInsets())
                    }
                } else {
                    ProgressView().padding(.vertical, 40).frame(maxWidth: .infinity).plainRow(EdgeInsets())
                }
            }

            if ap.error != nil || ap.notice != nil {
                VStack(spacing: 8) {
                    if let error = ap.error { IntegrationBanner(text: error, kind: .error).accessibilityIdentifier("autopilotError") }
                    if let notice = ap.notice { IntegrationBanner(text: notice).accessibilityIdentifier("autopilotNotice") }
                }
                .plainRow(EdgeInsets(top: 12, leading: 16, bottom: 0, trailing: 16))
            }

            grants

            Button {
                model.show(.permissions)
            } label: {
                TileRow("Permissions", glyph: .shield)
            }
            .buttonStyle(PressDimsStyle())
            .feedbackTap(.tap, feedback)
            .groupSlice(.only)
            .accessibilityIdentifier("trustPermissions")
            .plainRow(EdgeInsets(top: 12, leading: 0, bottom: 0, trailing: 0))

            autopilot

            StartingRuleChooser()
                .plainRow(EdgeInsets(top: 0, leading: 0, bottom: 8, trailing: 0))
        }
        .listStyle(.plain)
        .scrollContentBackground(.hidden)
        .contentMargins(.bottom, 24, for: .scrollContent)
        .ignoresSafeArea(edges: .top)
        // Only the pinned bars read it: scrolling redraws them, not the page.
        .onScrollGeometryChange(for: CGFloat.self) { $0.contentOffset.y + $0.contentInsets.top } action: { _, y in
            if mode != nil { fold.scroll(to: y) }
        }
        #if DEBUG
        // `-openStack`, `-trustScroll <points>` (screenshots): the running list open; Trust scrolled to a row.
        .task(id: mode) {
            let args = ProcessInfo.processInfo.arguments
            try? await Task.sleep(for: .milliseconds(1200))
            if args.contains("-openStack") { runningListOpen = true }

        }
        #endif
    }

    // MARK: A running mode

    /// What the running mode adds under its field: what a bypass or a lockdown decided, as a stack; what Overnight does
    /// with what is too risky.
    @ViewBuilder
    private func runningMiddle(_ mode: TrustRunning, _ s: AutopilotSettings) -> some View {
        switch mode {
        case .overnight:
            VStack(spacing: 0) {
                SectionHeader("Too risky", glyph: .warning)
                WhenRiskyPicker(selected: s.overnightWhenRisky) { risky in Task { await ap.setOvernightWhenRisky(risky) } }
                    .padding(.horizontal, 16)
            }
            .plainRow(EdgeInsets())
        case .bypass, .lockdown:
            let entries = mode.entries(model.activity, settings: s)
            if !entries.isEmpty {
                RunningStack(running: mode, entries: entries, open: $runningListOpen) { entry in
                    model.show(.activityDetail(entry.id), in: .activity)
                }
                .plainRow(EdgeInsets())
            }
        }
    }

    private func stop(_ mode: TrustRunning) {
        switch mode {
        case .bypass: Task { await ap.stopBypass() }
        case .overnight: Task { await ap.stopOvernight() }
        case .lockdown: Task { await ap.endLockdown() }
        }
    }

    // MARK: Grants

    /// "Grants  6  (+)", each running grant (swipe one to end it), then Ended.
    @ViewBuilder
    private var grants: some View {
        let running = running
        let endedCount = endedCount
        AddSectionHeader(title: "Grants", glyph: .key, count: running.count, addLabel: "New grant", addID: "newGrant") {
            feedback.defaultTap()
            model.show(.newGrant(token: 0))
        }
        .plainRow(EdgeInsets())
        .id("grants")

        if let grantError {
            FormBanner(text: grantError)
                .accessibilityIdentifier("resumeError")
                .plainRow(EdgeInsets(top: 0, leading: 16, bottom: 8, trailing: 16))
        }

        let count = max(running.count, 1) + (endedCount > 0 ? 1 : 0)
        if running.isEmpty {
            Text("Nothing runs without asking.")
                .textRole(.body)
                .foregroundStyle(Palette.inkMeta)
                .padding(.horizontal, 14)
                .frame(minHeight: 56, alignment: .leading)
                .groupSlice(.of(0, count: count), inset: 14)
                .accessibilityIdentifier("noGrants")
                .plainRow(EdgeInsets())
        }
        ForEach(Array(running.enumerated()), id: \.element.id) { i, grant in
            Button {
                model.show(.grantDetail(grant.id))
            } label: {
                LiveGrantRow(grant: grant)
            }
            .buttonStyle(PressDimsStyle())
            .feedbackTap(.tap, feedback)
            .groupSlice(.of(i, count: count), inset: 66)
            .accessibilityIdentifier("grant:\(grant.id)")
            .swipeActions(edge: .trailing, allowsFullSwipe: false) {
                Button { tapped { revoking = grant } } label: { Label("End now", systemImage: "nosign") }
                    .tint(Palette.danger)
            }
            .contextMenu {
                Button { tapped { model.show(.grantDetail(grant.id)) } } label: { Label("Open", systemImage: "key.horizontal") }
                Button(role: .destructive) { tapped { revoking = grant } } label: { Label("End now", systemImage: "nosign") }
            }
            .plainRow(EdgeInsets())
        }
        if endedCount > 0 {
            Button {
                model.show(.expiredGrants)
            } label: {
                TileRow("Ended", glyph: .timerOff, value: "\(endedCount)")
            }
            .buttonStyle(PressDimsStyle())
            .feedbackTap(.tap, feedback)
            .groupSlice(.of(count - 1, count: count), inset: 14)
            .accessibilityLabel("Ended, \(endedCount)")
            .accessibilityIdentifier("expiredHeader")
            .plainRow(EdgeInsets())
        }
    }

    // MARK: Autopilot

    /// "Autopilot  (+)": each profile, Try it, and the model.
    @ViewBuilder
    private var autopilot: some View {
        AddSectionHeader(title: "Autopilot", glyph: .autopilot, count: nil, addLabel: "New profile", addID: "newProfile") {
            feedback.play(.tap)
            newProfile = true
        }
        .plainRow(EdgeInsets())
        let profiles = ap.profiles
        let count = profiles.count + 2
        ForEach(Array(profiles.enumerated()), id: \.element.id) { i, profile in
            Button {
                model.openFromAutopilot(.autopilotProfile(profile.id))
            } label: {
                TileRow(untrusted(profile.name), value: "Profile") {
                    ProfileTile(profile: profile, size: 32)
                } trailing: {
                    EmptyView()
                }
            }
            .buttonStyle(PressDimsStyle())
            .feedbackTap(.tap, feedback)
            .groupSlice(.of(i, count: count))
            .accessibilityIdentifier("profile:\(profile.id)")
            .plainRow(EdgeInsets())
        }
        Button {
            model.openFromAutopilot(.tryIt(nil))
        } label: {
            TileRow("Try it", glyph: .flask)
        }
        .buttonStyle(PressDimsStyle())
        .feedbackTap(.tap, feedback)
        .groupSlice(.of(profiles.count, count: count))
        .accessibilityIdentifier("openTryIt")
        .plainRow(EdgeInsets())
        Button {
            feedback.play(.tap)
            modelOpen = true
        } label: {
            TileRow("Model", glyph: .chip) {
                ModelRowValue(ap: ap)
            }
        }
        .buttonStyle(PressDimsStyle())
        .groupSlice(.of(profiles.count + 1, count: count))
        .accessibilityIdentifier("modelRow")
        .plainRow(EdgeInsets())
    }

    private func download() {
        if ap.needsMobileDataConsent() { mobileDataAsk = true } else { ap.download() }
    }

    /// A menu or swipe item: its action, then the default tap (the page or dialog it opens has the sound).
    private func tapped(_ action: () -> Void) {
        action()
        feedback.defaultTap()
    }
}

#if DEBUG
/// `-trustScroll <points>` (screenshots): scrolls the list this sits in to that offset, a moment after it appears.
private struct DebugScroll: UIViewRepresentable {
    func makeUIView(context: Context) -> UIView {
        let view = UIView()
        let args = ProcessInfo.processInfo.arguments
        if let i = args.firstIndex(of: "-trustScroll"), i + 1 < args.count, let y = Double(args[i + 1]) {
            DispatchQueue.main.asyncAfter(deadline: .now() + 1.5) {
                var v: UIView? = view
                while let s = v, !(s is UIScrollView) { v = s.superview }
                (v as? UIScrollView)?.setContentOffset(CGPoint(x: 0, y: y - ((v as? UIScrollView)?.adjustedContentInset.top ?? 0)), animated: true)
            }
        }
        return view
    }

    func updateUIView(_ uiView: UIView, context: Context) {}
}
#endif
