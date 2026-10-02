import SwiftUI

/// One profile: how sure it must be, each kind of request on its way to running by itself (a ring fills toward
/// unlocking), and the profile's own settings.
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
        .navigationTitle(profile.map { untrusted($0.name) } ?? "Profile")
        .navigationBarTitleDisplayMode(.inline)
        .toolbar {
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
                    Task { await ap.renameProfile(profile.id, name: name, icon: icon) }
                }
            }
        }
        .alert(profile.map { "Forget what \(untrusted($0.name)) learned?" } ?? "", isPresented: $reset) {
            Button("Cancel", role: .cancel) {}
            Button("Forget", role: .destructive) { Task { await ap.resetProfile(profileId) } }
        } message: {
            Text("Its \(profile?.memoryCount ?? 0) remembered decisions go, every kind of request is locked again, and Autopilot starts learning from your next answer.")
        }
        .alert(profile.map { "Delete \(untrusted($0.name))?" } ?? "", isPresented: $delete) {
            Button("Cancel", role: .cancel) {}
            Button("Delete", role: .destructive) {
                Task { if await ap.deleteProfile(profileId) { model.back() } }
            }
        } message: {
            Text("What it learned is gone. AIs that used it move to the default profile.")
        }
        .alert(unlocking.map { "Let Auto approve \($0.label)?" } ?? "", isPresented: Binding(get: { unlocking != nil }, set: { if !$0 { unlocking = nil } })) {
            Button("Cancel", role: .cancel) { unlocking = nil }
            Button("Unlock") {
                if let cls = unlocking { Task { await ap.setClassLock(profileId, cls.classKey, locked: false) } }
                unlocking = nil
            }
        } message: {
            Text("Autopilot will approve these on its own when it is sure enough, before it has learned enough to unlock them by itself. The riskiest requests still wait for you.")
        }
    }

    private func content(_ profile: ProfileView) -> some View {
        VStack(alignment: .leading, spacing: 22) {
            ProfileHeader(profile: profile, connections: model.connections.filter { profile.connections.contains($0.id) }.map { untrusted($0.label) })
            if ap.error != nil || ap.notice != nil {
                VStack(spacing: 8) {
                    if let error = ap.error { IntegrationBanner(text: error, kind: .error).accessibilityIdentifier("profileError") }
                    if let notice = ap.notice { IntegrationBanner(text: notice).accessibilityIdentifier("profileNotice") }
                }
            }

            VStack(alignment: .leading, spacing: 8) {
                SectionHeader("How sure it must be").padding(.horizontal, 12)
                Picker("How sure it must be", selection: Binding(get: { profile.preset }, set: { preset in
                    feedback.play(.selection)
                    Task { await ap.setPreset(profile.id, preset) }
                })) {
                    ForEach(AutopilotText.presets, id: \.self) { p in
                        Text(AutopilotText.presetName(p)).tag(p)
                    }
                }
                .pickerStyle(.segmented)
                .controlSize(.large)
                .accessibilityIdentifier("preset")
                Text(AutopilotText.presetLine(profile.preset))
                    .font(RFont.sans(13))
                    .foregroundStyle(Palette.secondary)
                    .padding(.horizontal, 16)
                    .accessibilityIdentifier("presetLine")
            }

            AutopilotGroup(
                header: "Kinds of request",
                footer: "A kind runs by itself in Auto after 20 of your answers with 95% agreement, and never after Autopilot approved something you would have denied. Tap one to lock or unlock it yourself."
            ) {
                if profile.classes.isEmpty {
                    EmptyState(symbol: "sparkles", title: "Nothing learned yet", message: "Answer requests in Assisted or Auto: every approve and deny teaches this profile.")
                        .accessibilityIdentifier("noClasses")
                }
                ForEach(Array(profile.classes.enumerated()), id: \.element.classKey) { i, cls in
                    if i > 0 { RowDivider(inset: 84) }
                    ClassRow(cls: cls) { locked in
                        if locked == false {
                            unlocking = cls
                        } else {
                            Task { await ap.setClassLock(profile.id, cls.classKey, locked: locked) }
                        }
                    }
                }
            }

            AutopilotGroup(header: "Try it") {
                AutopilotRow(title: "Try this profile", subtitle: "See what it would do with a request you type", symbol: "flask", chevron: true, identifier: "tryProfile") {
                    model.push(.tryIt(profile.id))
                }
            }

            AutopilotGroup(header: "Profile") {
                if !profile.isDefault {
                    AutopilotRow(title: "Use for every AI by default", subtitle: "AIs without a profile of their own learn here", symbol: "star", identifier: "makeDefault") {
                        Task { await ap.makeDefault(profile.id) }
                    }
                    RowDivider(inset: 64)
                }
                AutopilotRow(
                    title: "Forget what it learned",
                    subtitle: "\(profile.memoryCount) remembered \(profile.memoryCount == 1 ? "decision" : "decisions"), and kinds you locked or unlocked",
                    symbol: "arrow.counterclockwise",
                    identifier: "resetProfile"
                ) { reset = true }
                if ap.profiles.count > 1 {
                    RowDivider(inset: 64)
                    AutopilotRow(title: "Delete profile", symbol: "trash", destructive: true, identifier: "deleteProfile") { delete = true }
                }
            }
        }
    }
}

private struct ProfileHeader: View {
    var profile: ProfileView
    var connections: [String]

    var body: some View {
        VStack(spacing: 0) {
            ProfileAvatar(profile: profile, size: 80)
            HStack(spacing: 8) {
                Text(untrusted(profile.name)).font(RFont.sans(24, .semibold)).foregroundStyle(Palette.text).lineLimit(1)
                if profile.isDefault {
                    StatusPill(text: "Default", tint: Palette.accent).accessibilityIdentifier("defaultTag")
                }
            }
            .padding(.top, 12)
            Text(learnsFrom)
                .font(RFont.sans(13.5))
                .foregroundStyle(Palette.secondary)
                .multilineTextAlignment(.center)
                .padding(.top, 4)
                .padding(.horizontal, 24)
            HStack(spacing: 10) {
                Stat(value: "\(profile.memoryCount)", label: "remembered").accessibilityIdentifier("statMemory")
                Stat(value: "\(profile.classes.filter(\.autoApprove).count) of \(profile.classes.count)", label: "on Auto")
                Stat(value: profile.trainedAt.map(NeighbourRow.relative) ?? "not yet", label: "trained")
            }
            .padding(.top, 18)
        }
        .frame(maxWidth: .infinity)
        .padding(.top, 8)
    }

    private var learnsFrom: String {
        if !connections.isEmpty { return "Learns from " + connections.joined(separator: ", ") }
        if profile.isDefault { return "Learns from every AI without a profile of its own" }
        return "No AI uses it yet: pick it on an AI's page"
    }
}

private struct Stat: View {
    var value: String
    var label: String

    var body: some View {
        VStack(spacing: 2) {
            Text(value).font(RFont.sans(18, .semibold)).foregroundStyle(Palette.text).lineLimit(1).minimumScaleFactor(0.7)
            Text(label).font(RFont.sans(12.5)).foregroundStyle(Palette.secondary).lineLimit(1)
        }
        .padding(.vertical, 14)
        .padding(.horizontal, 10)
        .frame(maxWidth: .infinity)
        .background(Palette.elevated, in: RoundedRectangle(cornerRadius: 18, style: .continuous))
        .accessibilityElement(children: .combine)
    }
}

/// One kind of request: a ring that fills with the user's answers toward unlocking (a tick once it runs by itself),
/// where it stands, and, opened, the lock.
private struct ClassRow: View {
    var cls: ClassView
    var onLock: (Bool?) -> Void
    @State private var open = false
    @Environment(\.feedback) private var feedback

    var body: some View {
        let locked = cls.manual == false
        let ring = locked ? Palette.tertiary : cls.autoApprove ? Palette.success : Palette.accent
        VStack(alignment: .leading, spacing: 0) {
            Button {
                withAnimation(.spring(response: 0.35, dampingFraction: 0.9)) { open.toggle() }
                feedback.play(.expand(open))
            } label: {
                HStack(spacing: 16) {
                    ProgressRing(fraction: locked ? 0 : AutopilotText.unlockProgress(cls), tint: ring, size: 52, stroke: 5) {
                        if locked {
                            Image(systemName: "lock.fill").font(.system(size: 16)).foregroundStyle(Palette.tertiary)
                        } else if cls.autoApprove {
                            Image(systemName: "checkmark").font(.system(size: 18, weight: .bold)).foregroundStyle(Palette.success)
                        } else {
                            Text("\(cls.decisions)").font(RFont.mono(14, .semibold)).foregroundStyle(Palette.text)
                        }
                    }
                    VStack(alignment: .leading, spacing: 2) {
                        Text(cls.label).font(RFont.sans(15.5, .semibold)).foregroundStyle(Palette.text).lineLimit(1)
                        Text(AutopilotText.classStatus(cls))
                            .font(RFont.sans(13, .medium))
                            .foregroundStyle(locked ? Palette.secondary : cls.autoApprove ? Palette.success : Palette.accent)
                            .fixedSize(horizontal: false, vertical: true)
                            .accessibilityIdentifier("classStatus:\(cls.classKey)")
                        Text(AutopilotText.classNumbers(cls)).font(RFont.sans(12.5)).foregroundStyle(Palette.tertiary).lineLimit(1)
                    }
                    Spacer(minLength: 8)
                    if cls.autoDeny { StatusPill(text: "Denies", tint: Palette.danger) }
                }
                .padding(.horizontal, 16)
                .padding(.vertical, 14)
                .contentShape(Rectangle())
            }
            .buttonStyle(RowPress())
            .accessibilityElement(children: .combine)
            .accessibilityAddTraits(.isButton)
            .accessibilityHint(open ? "Hides the lock" : "Shows the lock")
            .accessibilityIdentifier("class:\(cls.classKey)")
            if open {
                FlowLayout(spacing: 8) {
                    SelectChip(title: "Learn by itself", selected: cls.manual == nil, identifier: "lock:auto:\(cls.classKey)") { onLock(nil) }
                    SelectChip(title: "Always ask", selected: cls.manual == false, identifier: "lock:on:\(cls.classKey)") { onLock(true) }
                    SelectChip(title: "Unlock now", selected: cls.manual == true, identifier: "lock:off:\(cls.classKey)") { onLock(false) }
                }
                .padding(.leading, 84)
                .padding(.trailing, 16)
                .padding(.bottom, 14)
                .transition(.opacity.combined(with: .move(edge: .top)))
            }
        }
        .clipped()
    }
}
