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
                    .padding(.bottom, 32)
            } else if ap.loaded {
                EmptyState(symbol: "person.2", title: "Not found", message: "This profile no longer exists.")
                    .padding(.top, 60)
                    .accessibilityIdentifier("profileGone")
            } else {
                ProgressView().padding(.top, 60).frame(maxWidth: .infinity)
            }
        }
        .background(Palette.paper.ignoresSafeArea())
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
                        GlyphIcon(.pencil, size: 18).foregroundStyle(Palette.ink)
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
        .alert(unlocking.map { "Trust \($0.label) now?" } ?? "", isPresented: Binding(get: { unlocking != nil }, set: { if !$0 { unlocking = nil } })) {
            Button("Cancel", role: .cancel) { unlocking = nil }
            Button("Trust") {
                if let cls = unlocking {
                    feedback.quietClose()
                    Task { await ap.setClassLock(profileId, cls.classKey, locked: false) }
                }
                unlocking = nil
            }
        } message: {
            Text("Auto skips its cautious start for these. The riskiest still ask.")
        }
        .presentationFeedback(unlocking != nil)
    }

    private func content(_ profile: ProfileView) -> some View {
        VStack(alignment: .leading, spacing: 0) {
            ProfileHeader(profile: profile, ais: profileConnections(profile))
            if ap.error != nil || ap.notice != nil {
                VStack(spacing: 8) {
                    if let error = ap.error { IntegrationBanner(text: error, kind: .error).accessibilityIdentifier("profileError") }
                    if let notice = ap.notice { IntegrationBanner(text: notice).accessibilityIdentifier("profileNotice") }
                }
                .padding(.horizontal, 16)
                .padding(.top, 12)
            }

            SectionHeader("How sure", glyph: .shieldCheck) {
                HelpButton(text: AutopilotText.howSureHelp).frame(height: 20)
            }
            VStack(alignment: .leading, spacing: 12) {
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
            .padding(.horizontal, 16)
            .animation(.smooth(duration: 0.25), value: profile.preset)

            SectionHeader("Kinds", trailing: "\(profile.classes.count)")
            if profile.memoryCount > 0 {
                Text(AutopilotText.learningLine(profile))
                    .textRole(.sub)
                    .foregroundStyle(Palette.inkMeta)
                    .padding(.horizontal, 24)
                    .padding(.bottom, 8)
                    .accessibilityIdentifier("learningLine")
            }
            RaisedGroup(inset: 62) {
                if profile.classes.isEmpty {
                    Text("Nothing learned yet. Your answers teach it.")
                        .textRole(.body)
                        .foregroundStyle(Palette.inkMeta)
                        .padding(14)
                        .frame(maxWidth: .infinity, alignment: .leading)
                        .accessibilityIdentifier("noClasses")
                }
                ForEach(profile.classes, id: \.classKey) { cls in
                    KindRow(cls: cls) { locked in
                        if locked == false {
                            unlocking = cls
                        } else {
                            Task { await ap.setClassLock(profile.id, cls.classKey, locked: locked) }
                        }
                    }
                }
            }

            RaisedGroup(inset: 62) {
                Button { model.push(.tryIt(profile.id)) } label: { TileRow("Try this profile", glyph: .flask) }
                    .buttonStyle(PressDimsStyle())
                    .feedbackTap(.tap, feedback)
                    .accessibilityIdentifier("tryProfile")
            }
            .padding(.top, 22)

            RaisedGroup(inset: 62) {
                if !profile.isDefault {
                    Button { Task { await ap.makeDefault(profile.id) } } label: { TileRow("Use for every AI by default", glyph: .star, chevron: false) }
                        .buttonStyle(PressDimsStyle())
                        .accessibilityIdentifier("makeDefault")
                }
                Button { reset = true } label: { TileRow("Forget what it learned", glyph: .undo, chevron: false) }
                    .buttonStyle(PressDimsStyle())
                    .accessibilityIdentifier("resetProfile")
                if ap.profiles.count > 1 {
                    Button { delete = true } label: { TileRow("Delete profile", glyph: .trash, danger: true, chevron: false) }
                        .buttonStyle(PressDimsStyle())
                        .accessibilityIdentifier("deleteProfile")
                }
            }
            .padding(.top, 12)
        }
    }

    /// The AIs that use this profile, for their avatars.
    private func profileConnections(_ profile: ProfileView) -> [ConnectionView] {
        let names = AutopilotText.profileAis(profile, settings: model.autopilot, connections: model.connections.map { ($0.id, $0.label) })
        return model.connections.filter { c in names.contains(c.label) || names.contains(untrusted(c.label)) }
    }
}

/// The profile's tile and name (with Default), then the AIs that use it as overlapping avatars and how many.
private struct ProfileHeader: View {
    var profile: ProfileView
    var ais: [ConnectionView]

    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            HStack(spacing: 14) {
                ProfileTile(profile: profile, size: 52)
                Text(untrusted(profile.name))
                    .textRole(.title)
                    .foregroundStyle(Palette.ink)
                    .lineLimit(2)
                    .minimumScaleFactor(0.8)
                    .accessibilityAddTraits(.isHeader)
                Spacer(minLength: 8)
                if profile.isDefault {
                    Pill("Default", tone: .violet).accessibilityIdentifier("defaultTag")
                }
            }
            HStack(spacing: 10) {
                HStack(spacing: -8) {
                    ForEach(ais.prefix(7), id: \.id) { c in
                        ConnectionAvatar(connection: c, size: 30)
                            .padding(2)
                            .background(Palette.paper, in: Circle())
                    }
                }
                .accessibilityHidden(true)
                Text(ais.isEmpty ? "No AI yet" : ais.count == 1 ? "1 AI" : "\(ais.count) AIs")
                    .textRole(.meta)
                    .foregroundStyle(Palette.inkMeta)
            }
            .accessibilityElement(children: .ignore)
            .accessibilityLabel(ais.isEmpty ? "No AI yet" : ais.map { untrustedName($0.label, limit: 40) }.joined(separator: ", "))
            .accessibilityIdentifier("statMemory")
        }
        .padding(.horizontal, 20)
        .padding(.top, 4)
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
                    .foregroundStyle(Palette.inkMeta)
                    .fixedSize(horizontal: false, vertical: true)
                    .accessibilityIdentifier("replay")
            }
        }
        .padding(.horizontal, 16)
        .padding(.vertical, 14)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(Palette.paperRaised, in: RoundedRectangle(cornerRadius: 20, style: .continuous))
        .overlay(RoundedRectangle(cornerRadius: 20, style: .continuous).strokeBorder(Palette.rule, lineWidth: 0.5))
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
            if part.strong { piece.foregroundColor = Palette.ink }
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
                Text(title).font(RFont.sans(14.5)).foregroundStyle(Palette.ink)
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
                    Rectangle().fill(Palette.inkMeta)
                        .frame(width: 2, height: 16)
                        .offset(x: x(lo) - 1)
                    Circle().fill(Color.white)
                        .frame(width: 22, height: 22)
                        .shadow(color: .black.opacity(0.35), radius: 4, y: 1)
                        .offset(x: x(value) - 11)
                }
                .frame(width: w, height: 28)
                .contentShape(Rectangle())
                // The page scrolls: a tap sets the threshold where it lands, and a drag moves it only when the finger
                // goes sideways (SidewaysPan), so a scroll that starts on the track scrolls.
                .onTapGesture { point in
                    set(point.x, width: w)
                    onCommit()
                }
                .gesture(SidewaysPan(onChanged: { drag in
                    dragging = true
                    set(drag.location.x, width: w)
                }, onEnded: { _, _ in
                    dragging = false
                    onCommit()
                }))
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

    /// The threshold under `x` on a track `width` wide, with a detent tick for each step.
    private func set(_ x: CGFloat, width: CGFloat) {
        let p = min(max(Int((x / max(width, 1) * 100).rounded()), lo), hi)
        if p != value {
            value = p
            feedback.play(.detent, step: p - lo)
        }
    }
}

/// One kind of request (the Autopilot board, 05): its request icon, its name, its state as a tag (Auto, Asks, Paused,
/// Locked); opened, its numbers and the lock.
private struct KindRow: View {
    var cls: ClassView
    var onLock: (Bool?) -> Void
    @State private var open = false
    @Environment(\.feedback) private var feedback

    var body: some View {
        let state = AutopilotText.kindState(cls)
        VStack(alignment: .leading, spacing: 0) {
            Button {
                withAnimation(.spring(response: 0.35, dampingFraction: 0.9)) { open.toggle() }
                feedback.play(.expand(open))
            } label: {
                HStack(spacing: 12) {
                    RequestIcon(spec: Self.icon(cls), size: 36, ring: Palette.paperRaised)
                    Text(cls.label).textRole(.row).foregroundStyle(Palette.ink).lineLimit(1)
                        .frame(maxWidth: .infinity, alignment: .leading)
                    KindTag(state: state)
                        .accessibilityIdentifier("kindLine:\(cls.classKey)")
                    RowChevron().rotationEffect(.degrees(open ? 90 : 0))
                }
                .padding(.horizontal, 14)
                .padding(.vertical, 10)
                .frame(minHeight: 58)
                .contentShape(Rectangle())
            }
            .buttonStyle(PressDimsStyle())
            .accessibilityElement(children: .combine)
            .accessibilityAddTraits(.isButton)
            .accessibilityHint(open ? "Hides the lock" : "Shows the lock")
            .accessibilityIdentifier("class:\(cls.classKey)")
            if open {
                VStack(alignment: .leading, spacing: 10) {
                    VStack(alignment: .leading, spacing: 2) {
                        Text(AutopilotText.classStatus(cls))
                            .font(RFont.sans(13, .medium))
                            .foregroundStyle(state == .auto ? Palette.violet : state == .asks ? Palette.kindRead : Palette.ink2)
                            .fixedSize(horizontal: false, vertical: true)
                            .accessibilityIdentifier("classStatus:\(cls.classKey)")
                        Text(AutopilotText.classNumbers(cls)).textRole(.sub).foregroundStyle(Palette.inkMeta)
                    }
                    FlowRow(spacing: 8) {
                        AutopilotChip(title: "Learn by itself", selected: cls.manual == nil, identifier: "lock:auto:\(cls.classKey)") { onLock(nil) }
                        AutopilotChip(title: "Always ask", selected: cls.manual == false, identifier: "lock:on:\(cls.classKey)") { onLock(true) }
                        AutopilotChip(title: "Trust now", selected: cls.manual == true, identifier: "lock:off:\(cls.classKey)") { onLock(false) }
                    }
                }
                .padding(.leading, 62)
                .padding(.trailing, 16)
                .padding(.bottom, 14)
                .transition(.opacity.combined(with: .move(edge: .top)))
            }
        }
        .clipped()
    }

    /// The kind's request icon from its key ("github/write/push"): the service's mark, the action's badge.
    static func icon(_ cls: ClassView) -> AskIcon {
        let parts = cls.classKey.split(separator: "/").map(String.init)
        let provider = parts.first ?? ""
        let action = parts.count > 2 ? parts[2] : parts.count > 1 ? parts[1] : ""
        let kind = parts.count > 1 ? ActionKind.of(parts[1]) : .other
        return AskLabels.icon(provider: provider, action: action, field: FieldKind.of(kind))
    }
}

/// A kind's state as a glyph and a word on its tint (the boards' `ktag`).
private struct KindTag: View {
    var state: AutopilotText.KindState

    var body: some View {
        let (word, tint): (String, Color) = switch state {
        case .auto: ("Auto", Palette.violet)
        case .asks: ("Asks", Palette.kindRead)
        case .paused: ("Paused", Palette.ink2)
        case .locked: ("Locked", Palette.warning)
        }
        HStack(spacing: 4) {
            switch state {
            case .auto: Bolt(size: 12)
            case .asks: GlyphIcon(.help, size: 12)
            case .paused: GlyphIcon(.pause, size: 12)
            case .locked: GlyphIcon(.lock, size: 12)
            }
            Text(word).font(RFont.fixedSans(12.5, .semibold))
        }
        .foregroundStyle(tint)
        .padding(.horizontal, 9)
        .frame(height: 24)
        .background(tint.opacity(0.12), in: Capsule())
        .fixedSize()
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(word)
    }
}
