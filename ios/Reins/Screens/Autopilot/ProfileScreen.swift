import SwiftUI

/// One profile: its numbers, how sure it must be (a preset or thresholds of the user's own), each kind of request on
/// its way to running by itself (a ring fills toward Auto), and the profile's own settings.
struct ProfileScreen: View {
    var profileId: String

    @Environment(AppModel.self) private var model
    @Environment(\.feedback) private var feedback
    @State private var ap = AutopilotModel()
    @State private var rename = false
    @State private var reset = false
    @State private var delete = false
    @State private var unlocking: ClassView?

    var body: some View {
        let profile = ap.profile(profileId)
        ScrollView {
            if let profile {
                content(profile)
                    .frame(maxWidth: 680)
                    .frame(maxWidth: .infinity)
                    .padding(.horizontal, 16)
                    .padding(.bottom, 32)
            } else if ap.loaded {
                EmptyState(symbol: "person.2", title: "Not found", message: "This profile no longer exists.")
                    .padding(.top, 60)
                    .accessibilityIdentifier("profileGone")
            } else {
                ProgressView().padding(.top, 60).frame(maxWidth: .infinity)
            }
        }
        .background(Palette.background.ignoresSafeArea())
        .navigationTitle("")
        .navigationBarTitleDisplayMode(.inline)
        .toolbar {
            // The name is the page's own heading, beside its emoji.
            ToolbarItem(placement: .principal) { EmptyView() }
            if profile != nil {
                ToolbarItem(placement: .primaryAction) {
                    Button {
                        feedback.play(.tap)
                        rename = true
                    } label: {
                        Image(systemName: "pencil")
                    }
                    .accessibilityLabel("Rename")
                    .accessibilityIdentifier("renameProfile")
                }
            }
        }
        .refreshable {
            feedback.play(.refresh)
            await ap.refresh()
        }
        .task(id: profileId) {
            ap.bind(model)
            await ap.refresh()
        }
        .sheet(isPresented: $rename) {
            if let profile {
                ProfileEditorSheet(title: "Rename profile", initialName: profile.name, initialIcon: profile.icon, confirmLabel: "Save") { name, icon in
                    feedback.quietClose()
                    Task { await ap.renameProfile(profile.id, name: name, icon: icon) }
                }
            }
        }
        .presentationFeedback(rename)
        .alert(profile.map { "Forget what \(untrusted($0.name)) learned?" } ?? "", isPresented: $reset) {
            Button("Cancel", role: .cancel) {}
            Button("Forget", role: .destructive) {
                feedback.quietClose()
                Task { await ap.resetProfile(profileId) }
            }
        } message: {
            Text("Its \(profile?.memoryCount ?? 0) decisions go and it starts over.")
        }
        .presentationFeedback(reset)
        .alert(profile.map { "Delete \(untrusted($0.name))?" } ?? "", isPresented: $delete) {
            Button("Cancel", role: .cancel) {}
            Button("Delete", role: .destructive) {
                feedback.quietClose()
                Task { if await ap.deleteProfile(profileId) { model.back() } }
            }
        } message: {
            Text("Its AIs move to the default profile.")
        }
        .presentationFeedback(delete)
        .alert(unlocking.map { "Let Auto approve \($0.label)?" } ?? "", isPresented: Binding(get: { unlocking != nil }, set: { if !$0 { unlocking = nil } })) {
            Button("Cancel", role: .cancel) { unlocking = nil }
            Button("Unlock") {
                if let cls = unlocking {
                    feedback.quietClose()
                    Task { await ap.setClassLock(profileId, cls.classKey, locked: false) }
                }
                unlocking = nil
            }
        } message: {
            Text("Auto approves these when it is sure. The riskiest still ask.")
        }
        .presentationFeedback(unlocking != nil)
    }

    private func content(_ profile: ProfileView) -> some View {
        VStack(alignment: .leading, spacing: 22) {
            ProfileHeader(profile: profile, ais: AutopilotText.profileAis(
                profile, settings: model.autopilot, connections: model.connections.map { ($0.id, $0.label) }
            ))
            if ap.error != nil || ap.notice != nil {
                VStack(spacing: 8) {
                    if let error = ap.error { IntegrationBanner(text: error, kind: .error).accessibilityIdentifier("profileError") }
                    if let notice = ap.notice { IntegrationBanner(text: notice).accessibilityIdentifier("profileNotice") }
                }
            }

            VStack(alignment: .leading, spacing: 10) {
                HelpHeader(title: "How sure before it acts", help: AutopilotText.howSureHelp).padding(.horizontal, 12)
                Chooser(AutopilotText.presets.map { p in
                    let name = AutopilotText.presetName(p)
                    return Chooser.Segment(id: "preset:\(name.lowercased())", title: name, selected: profile.preset == p) {
                        guard p != profile.preset else { return }
                        if p == .custom {
                            // Custom starts from the thresholds in use.
                            Task { await ap.setThresholds(profile.id, approveAt: profile.approveAt, denyAt: profile.denyAt) }
                        } else {
                            Task { await ap.setPreset(profile.id, p) }
                        }
                    }
                })
                .accessibilityElement(children: .contain)
                .accessibilityLabel("How sure")
                .accessibilityValue(AutopilotText.presetLine(profile))
                .accessibilityIdentifier("preset")
                if profile.preset == .custom {
                    ThresholdCard(profile: profile, ap: ap)
                        .transition(.opacity.combined(with: .move(edge: .top)))
                }
            }
            .animation(.smooth(duration: 0.25), value: profile.preset)

            AutopilotGroup(header: "Kinds of request") {
                if profile.classes.isEmpty {
                    EmptyState(symbol: "sparkles", title: "Nothing learned yet", message: "Your answers teach it.")
                        .accessibilityIdentifier("noClasses")
                }
                ForEach(Array(profile.classes.enumerated()), id: \.element.classKey) { i, cls in
                    if i > 0 { RowDivider(inset: 60) }
                    KindRow(cls: cls) { locked in
                        if locked == false {
                            unlocking = cls
                        } else {
                            Task { await ap.setClassLock(profile.id, cls.classKey, locked: locked) }
                        }
                    }
                }
            }

            AutopilotGroup(header: nil) {
                AutopilotRow(title: "Try this profile", symbol: "flask", tint: Palette.send, chevron: true, identifier: "tryProfile") {
                    model.push(.tryIt(profile.id))
                }
            }

            AutopilotGroup(header: nil) {
                if !profile.isDefault {
                    AutopilotRow(title: "Use for every AI by default", symbol: "star", identifier: "makeDefault") {
                        Task { await ap.makeDefault(profile.id) }
                    }
                    RowDivider(inset: 64)
                }
                AutopilotRow(title: "Forget what it learned", symbol: "arrow.counterclockwise", identifier: "resetProfile") { reset = true }
                if ap.profiles.count > 1 {
                    RowDivider(inset: 64)
                    AutopilotRow(title: "Delete profile", symbol: "trash", destructive: true, identifier: "deleteProfile") { delete = true }
                }
            }
        }
    }
}

/// The emoji, the name, and the AIs that use it; then its numbers.
private struct ProfileHeader: View {
    var profile: ProfileView
    var ais: [String]

    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            HStack(spacing: 14) {
                ProfileTile(profile: profile, size: 58)
                VStack(alignment: .leading, spacing: 2) {
                    HStack(spacing: 8) {
                        Text(untrusted(profile.name))
                            .font(RFont.serif(40))
                            .foregroundStyle(Palette.ink)
                            .lineLimit(1)
                            .minimumScaleFactor(0.7)
                            .accessibilityAddTraits(.isHeader)
                        if profile.isDefault {
                            StatusPill(text: "Default", tint: Palette.accent).accessibilityIdentifier("defaultTag")
                        }
                    }
                    Text(ais.isEmpty ? "No AI yet" : ais.map { untrustedName($0, limit: 40) }.joined(separator: ", "))
                        .textRole(.meta)
                        .foregroundStyle(Palette.inkMeta)
                        .lineLimit(2)
                }
            }
            .padding(.horizontal, 4)
            // Its numbers in the serif between two hairlines, as Trust's week.
            HStack(alignment: .top, spacing: 8) {
                stat("\(profile.memoryCount)", profile.memoryCount == 1 ? "decision" : "decisions").accessibilityIdentifier("statMemory")
                stat(AutopilotText.share(profile.agreement), "matched you", tint: profile.agreement == nil ? Palette.ink : Palette.success)
                    .accessibilityIdentifier("statAgreement")
                stat("\(profile.classes.filter(\.autoApprove).count)", "kinds on Auto", suffix: "/\(profile.classes.count)")
                    .accessibilityIdentifier("statKinds")
            }
            .padding(.vertical, 12)
            .overlay(alignment: .top) { Rectangle().fill(Palette.rule).frame(height: 1) }
            .overlay(alignment: .bottom) { Rectangle().fill(Palette.rule).frame(height: 1) }
        }
        .padding(.top, 4)
    }
}

private extension ProfileHeader {
    func stat(_ value: String, _ label: String, tint: Color = Palette.ink, suffix: String? = nil) -> some View {
        VStack(alignment: .leading, spacing: 0) {
            HStack(alignment: .lastTextBaseline, spacing: 1) {
                Text(value).font(RFont.serif(32)).foregroundStyle(tint)
                if let suffix { Text(suffix).font(RFont.serif(20)).foregroundStyle(Palette.inkMeta) }
            }
            .lineLimit(1)
            Text(label).textRole(.meta).foregroundStyle(Palette.inkMeta).lineLimit(1)
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .accessibilityElement(children: .combine)
    }
}

/// Custom thresholds: "Approve at" and "Deny at", 80% to 99%, saved when a drag ends, and what the last 30 days would
/// have been at them, read again (shortly after) as they move.
private struct ThresholdCard: View {
    var profile: ProfileView
    var ap: AutopilotModel
    @State private var approve = 95
    @State private var deny = 90
    @State private var dragging = false

    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            ThresholdSlider(title: "Approve at", value: $approve, tint: Palette.success, identifier: "approveAt", dragging: $dragging, onCommit: save)
            ThresholdSlider(title: "Deny at", value: $deny, tint: Palette.danger, identifier: "denyAt", dragging: $dragging, onCommit: save)
            if let replay = ap.replay {
                Text(replayText(replay))
                    .font(RFont.sans(12.5))
                    .foregroundStyle(Palette.tertiary)
                    .fixedSize(horizontal: false, vertical: true)
                    .accessibilityIdentifier("replay")
            }
        }
        .padding(.horizontal, 16)
        .padding(.vertical, 14)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(Palette.elevated, in: RoundedRectangle(cornerRadius: 20, style: .continuous))
        .overlay(RoundedRectangle(cornerRadius: 20, style: .continuous).strokeBorder(Palette.hairline, lineWidth: 0.5))
        .onAppear(perform: follow)
        .onChange(of: profile.approveAt) { if !dragging { follow() } }
        .onChange(of: profile.denyAt) { if !dragging { follow() } }
        .task(id: "\(profile.id):\(approve):\(deny)") {
            // Read again a moment after the thumbs stop, not at every step.
            try? await Task.sleep(for: .milliseconds(250))
            guard !Task.isCancelled else { return }
            await ap.loadReplay(profile.id, approveAt: Float(approve) / 100, denyAt: Float(deny) / 100)
        }
    }

    private func follow() {
        approve = AutopilotText.thresholdPercent(profile.approveAt)
        deny = AutopilotText.thresholdPercent(profile.denyAt)
    }

    private func save() {
        let (a, d) = (Float(approve) / 100, Float(deny) / 100)
        guard a != AutopilotModel.clampThreshold(profile.approveAt) || d != AutopilotModel.clampThreshold(profile.denyAt) else { return }
        Task { await ap.setThresholds(profile.id, approveAt: a, denyAt: d) }
    }

    /// The replay line with its counts in the text colour.
    private func replayText(_ r: ThresholdReplay) -> AttributedString {
        AutopilotText.replayParts(r).reduce(into: AttributedString()) { text, part in
            var piece = AttributedString(part.text)
            if part.strong { piece.foregroundColor = Palette.text }
            text.append(piece)
        }
    }
}

/// A threshold in whole percent: the track stands for 0 to 100, a tick marks the 80% floor, the active part runs
/// from it to the thumb, and the thumb stays within 80 to 99. Adjustable for VoiceOver; `onCommit` when a drag ends
/// (or VoiceOver changed it).
private struct ThresholdSlider: View {
    var title: String
    @Binding var value: Int
    var tint: Color
    var identifier: String
    @Binding var dragging: Bool
    var onCommit: () -> Void
    @Environment(\.feedback) private var feedback

    private let lo = AutopilotText.minThresholdPercent
    private let hi = AutopilotText.maxThresholdPercent

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            HStack {
                Text(title).font(RFont.sans(14.5)).foregroundStyle(Palette.text)
                Spacer()
                Text("\(value)%").font(RFont.mono(14, .semibold)).foregroundStyle(tint).monospacedDigit()
            }
            GeometryReader { geo in
                let w = geo.size.width
                let x = { (p: Int) in w * CGFloat(p) / 100 }
                ZStack(alignment: .leading) {
                    Capsule().fill(AutopilotStyle.track).frame(height: 6)
                    Capsule().fill(tint)
                        .frame(width: max(x(value) - x(lo), 0), height: 6)
                        .offset(x: x(lo))
                    Rectangle().fill(Palette.tertiary)
                        .frame(width: 2, height: 16)
                        .offset(x: x(lo) - 1)
                    Circle().fill(Color.white)
                        .frame(width: 22, height: 22)
                        .shadow(color: .black.opacity(0.35), radius: 4, y: 1)
                        .offset(x: x(value) - 11)
                }
                .frame(width: w, height: 28)
                .contentShape(Rectangle())
                .gesture(
                    DragGesture(minimumDistance: 0)
                        .onChanged { drag in
                            dragging = true
                            let p = min(max(Int((drag.location.x / max(w, 1) * 100).rounded()), lo), hi)
                            if p != value {
                                value = p
                                feedback.play(.detent, step: p - lo)
                            }
                        }
                        .onEnded { _ in
                            dragging = false
                            onCommit()
                        }
                )
            }
            .frame(height: 28)
        }
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(title)
        .accessibilityValue("\(value) percent")
        .accessibilityAdjustableAction { direction in
            switch direction {
            case .increment: value = min(value + 1, hi)
            case .decrement: value = max(value - 1, lo)
            @unknown default: return
            }
            onCommit()
        }
        .accessibilityIdentifier(identifier)
    }
}

/// One kind of request: a ring that fills with the user's answers toward Auto (full and green once on it, a lock when
/// it always asks), where it stands, and, opened, its numbers and the lock.
private struct KindRow: View {
    var cls: ClassView
    var onLock: (Bool?) -> Void
    @State private var open = false
    @Environment(\.feedback) private var feedback

    var body: some View {
        let locked = cls.manual == false
        VStack(alignment: .leading, spacing: 0) {
            Button {
                withAnimation(.spring(response: 0.35, dampingFraction: 0.9)) { open.toggle() }
                feedback.play(.expand(open))
            } label: {
                HStack(spacing: 12) {
                    if locked {
                        Circle().fill(Palette.chip)
                            .frame(width: 34, height: 34)
                            .overlay { Image(systemName: "lock.fill").font(.system(size: 13)).foregroundStyle(Palette.secondary) }
                    } else {
                        ProgressRing(
                            fraction: AutopilotText.unlockProgress(cls),
                            tint: cls.autoApprove ? Palette.success : Palette.search, size: 34, stroke: 3.5
                        )
                    }
                    VStack(alignment: .leading, spacing: 2) {
                        Text(cls.label).font(RFont.sans(15, .medium)).foregroundStyle(Palette.text).lineLimit(1)
                        Text(AutopilotText.kindLine(cls))
                            .font(RFont.sans(12.5))
                            .foregroundStyle(Palette.tertiary)
                            .lineLimit(1)
                            .accessibilityIdentifier("kindLine:\(cls.classKey)")
                    }
                    Spacer(minLength: 8)
                    if cls.autoApprove && !locked {
                        StatusPill(text: "Auto", tint: Palette.success)
                    } else {
                        Chevron().rotationEffect(.degrees(open ? 90 : 0))
                    }
                }
                .padding(.horizontal, 14)
                .padding(.vertical, 12)
                .contentShape(Rectangle())
            }
            .buttonStyle(RowPress())
            .accessibilityElement(children: .combine)
            .accessibilityAddTraits(.isButton)
            .accessibilityHint(open ? "Hides the lock" : "Shows the lock")
            .accessibilityIdentifier("class:\(cls.classKey)")
            if open {
                VStack(alignment: .leading, spacing: 10) {
                    VStack(alignment: .leading, spacing: 2) {
                        Text(AutopilotText.classStatus(cls))
                            .font(RFont.sans(13, .medium))
                            .foregroundStyle(locked ? Palette.secondary : cls.autoApprove ? Palette.success : Palette.search)
                            .fixedSize(horizontal: false, vertical: true)
                            .accessibilityIdentifier("classStatus:\(cls.classKey)")
                        Text(AutopilotText.classNumbers(cls)).font(RFont.sans(12.5)).foregroundStyle(Palette.tertiary)
                    }
                    FlowRow(spacing: 8) {
                        AutopilotChip(title: "Learn by itself", selected: cls.manual == nil, identifier: "lock:auto:\(cls.classKey)") { onLock(nil) }
                        AutopilotChip(title: "Always ask", selected: cls.manual == false, identifier: "lock:on:\(cls.classKey)") { onLock(true) }
                        AutopilotChip(title: "Unlock now", selected: cls.manual == true, identifier: "lock:off:\(cls.classKey)") { onLock(false) }
                    }
                }
                .padding(.leading, 60)
                .padding(.trailing, 16)
                .padding(.bottom, 14)
                .transition(.opacity.combined(with: .move(edge: .top)))
            }
        }
        .clipped()
    }
}
