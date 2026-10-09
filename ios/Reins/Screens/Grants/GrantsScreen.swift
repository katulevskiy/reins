import SwiftUI

/// The Grants section: the permissions that are running, each with its time left. The ones that ended (expired, used
/// up or deleted) wait behind a Liquid Glass pill above the tab bar, in a panel that rises over the list when opened.
/// Swiping a running grant deletes it (it can be resumed later); ended ones are resumed or deleted for good.
struct GrantsScreen: View {
    @Environment(AppModel.self) private var model
    @Environment(\.feedback) private var feedback
    @State private var resuming: GrantPick?
    @State private var deleting: GrantView?
    @State private var revoking: GrantView?
    @State private var error: String?
    @State private var panelContentHeight: CGFloat = 0

    private var running: [GrantView] { model.grants.filter(\.active).sorted { $0.createdAt > $1.createdAt } }
    private var ended: [GrantView] { model.grants.filter { !$0.active }.sorted { $0.createdAt > $1.createdAt } }

    var body: some View {
        let running = running
        let ended = ended
        let open = model.expiredOpen && !ended.isEmpty
        GeometryReader { geo in
            List {
                PageHeader(title: "Grants") {
                    Button {
                        model.show(.newGrant(token: 0))
                    } label: {
                        Label("New grant", systemImage: "plus")
                    }
                    .buttonStyle(SmallCapsuleStyle(kind: .primary))
                    .feedbackTap(.tap, feedback)
                    .accessibilityIdentifier("newGrant")
                }
                .padding(.top, 8)
                .plainRow(EdgeInsets(top: 0, leading: 20, bottom: 6, trailing: 16))

                if let error {
                    FormBanner(text: error)
                        .accessibilityIdentifier("resumeError")
                        .plainRow(EdgeInsets(top: 6, leading: 16, bottom: 6, trailing: 16))
                }

                if running.isEmpty {
                    EmptyState(
                        symbol: "key.horizontal",
                        title: model.grants.isEmpty ? "No grants" : "No active grants",
                        message: model.grants.isEmpty
                            ? "Allow something for a while from an approval, when an AI asks, or create one yourself."
                            : "Everything asks you first. Resume an expired one below, or create a new grant."
                    )
                    .padding(.top, 40)
                    .accessibilityIdentifier("noGrants")
                    .plainRow(EdgeInsets())
                }

                ForEach(running, id: \.id) { grant in
                    Button {
                        model.show(.grantDetail(grant.id))
                    } label: {
                        LiveGrantTile(grant: grant)
                    }
                    .buttonStyle(.plain)
                    .feedbackTap(.tap, feedback)
                    .accessibilityIdentifier("grant:\(grant.id)")
                    .swipeActions(edge: .trailing, allowsFullSwipe: false) {
                        Button { tapped { revoking = grant } } label: { Label("Delete", systemImage: "trash") }
                            .tint(Palette.danger)
                    }
                    .contextMenu {
                        Button { tapped { model.show(.grantDetail(grant.id)) } } label: { Label("Open", systemImage: "key.horizontal") }
                        Button(role: .destructive) { tapped { revoking = grant } } label: { Label("Delete grant", systemImage: "trash") }
                    }
                    .plainRow(EdgeInsets(top: 6, leading: 16, bottom: 6, trailing: 16))
                }

                // The starting rule for new AIs.
                StartingRuleChooser()
                    .padding(.top, 18)
                    .plainRow(EdgeInsets(top: 6, leading: 16, bottom: 6, trailing: 16))
            }
            .listStyle(.plain)
            .scrollContentBackground(.hidden)
            .contentMargins(.bottom, 24, for: .scrollContent)
            .refreshable {
                await model.refreshPending()
                feedback.play(.refresh)
            }
            .overlay {
                if open {
                    Palette.scrim
                        .ignoresSafeArea()
                        .onTapGesture { toggleExpired() }
                        .transition(.opacity)
                        .accessibilityHidden(true)
                }
            }
            .overlay(alignment: .bottom) {
                if open {
                    expiredPanel(ended, maxHeight: geo.size.height * 0.58)
                        .transition(.move(edge: .bottom).combined(with: .opacity))
                }
            }
            .safeAreaInset(edge: .bottom) {
                if !ended.isEmpty { expiredPill(count: ended.count, open: open) }
            }
            .animation(.smooth(duration: 0.34), value: open)
        }
        .pageBackground()
        .navigationTitle("Grants")
        .rootNavigationBar()
        .sheet(item: $resuming) { pick in
            let grant = pick.grant
            ResumeSheet(grant: grant) { seconds, standing in
                error = nil
                feedback.quietClose()
                Task { error = await model.resumeGrant(grant.id, seconds: seconds, standing: standing) }
            }
            .environment(model)
            .environment(\.feedback, feedback)
        }
        .presentationFeedback(resuming != nil)
        .confirmationDialog(
            "Delete this grant for good?",
            isPresented: Binding(get: { deleting != nil }, set: { if !$0 { deleting = nil } }),
            titleVisibility: .visible,
            presenting: deleting
        ) { grant in
            Button("Delete", role: .destructive) {
                error = nil
                feedback.quietClose()
                Task { error = await model.deleteGrant(grant.id) }
            }
        } message: { grant in
            Text(untrusted(grant.summary) + "\n\nIt cannot be resumed afterwards.")
        }
        .presentationFeedback(deleting != nil)
        .confirmationDialog(
            "Delete this grant?",
            isPresented: Binding(get: { revoking != nil }, set: { if !$0 { revoking = nil } }),
            titleVisibility: .visible,
            presenting: revoking
        ) { grant in
            Button("Delete", role: .destructive) {
                error = nil
                feedback.quietClose()
                Task { error = await model.revokeGrant(grant.id) }
            }
        } message: { grant in
            Text(untrusted(grant.summary) + "\n\nYou can resume it later from the Expired list.")
        }
        .presentationFeedback(revoking != nil)
    }

    /// A menu or swipe item: its action, then the default tap (the page or dialog it opens has the sound).
    private func tapped(_ action: () -> Void) {
        action()
        feedback.defaultTap()
    }

    private func toggleExpired() {
        feedback.play(.expand(!model.expiredOpen))
        withAnimation(.smooth(duration: 0.34)) { model.expiredOpen.toggle() }
    }

    /// The header pill that always sits just above the tab bar.
    private func expiredPill(count: Int, open: Bool) -> some View {
        Button(action: toggleExpired) {
            HStack {
                Text("Expired · \(count)")
                    .font(RFont.sans(15, .semibold))
                    .foregroundStyle(Palette.text)
                Spacer()
                Image(systemName: "chevron.up")
                    .font(.system(size: 14, weight: .semibold))
                    .foregroundStyle(Palette.secondary)
                    .rotationEffect(.degrees(open ? 180 : 0))
            }
            .padding(.horizontal, 20)
            .frame(maxWidth: .infinity, minHeight: 52)
            .contentShape(RoundedRectangle(cornerRadius: 26, style: .continuous))
        }
        .buttonStyle(.plain)
        .glassEffect(.regular.interactive(), in: RoundedRectangle(cornerRadius: 26, style: .continuous))
        .padding(.horizontal, 16)
        .padding(.bottom, 8)
        .accessibilityIdentifier("expiredHeader")
        .accessibilityLabel("Expired, \(count)")
        .accessibilityValue(open ? "Open" : "Closed")
        .accessibilityHint(open ? "Closes the list of ended grants" : "Shows the grants that ended")
    }

    /// The ended grants, in a panel that rises out from behind the pill.
    private func expiredPanel(_ ended: [GrantView], maxHeight: CGFloat) -> some View {
        ScrollView {
            VStack(spacing: 0) {
                ForEach(Array(ended.enumerated()), id: \.element.id) { index, grant in
                    if index > 0 { Divider().padding(.leading, 74) }
                    EndedGrantRow(
                        grant: grant,
                        onOpen: {
                            model.show(.grantDetail(grant.id))
                            feedback.defaultTap()
                        },
                        onResume: { resuming = GrantPick(grant) },
                        onDelete: { deleting = grant }
                    )
                    .contextMenu {
                        Button { tapped { model.show(.grantDetail(grant.id)) } } label: { Label("Open", systemImage: "key.horizontal") }
                        Button { tapped { resuming = GrantPick(grant) } } label: { Label("Resume", systemImage: "arrow.clockwise") }
                        Button(role: .destructive) { tapped { deleting = grant } } label: { Label("Delete for good", systemImage: "trash") }
                    }
                }
            }
            .padding(.vertical, 4)
            .onGeometryChange(for: CGFloat.self, of: \.size.height) { panelContentHeight = $0 }
        }
        .scrollBounceBehavior(.basedOnSize)
        .frame(height: min(max(panelContentHeight, 80), maxHeight))
        // Nearly opaque: the rows must read cleanly over the tiles below.
        .background(Palette.plate, in: RoundedRectangle(cornerRadius: 26, style: .continuous))
        .clipShape(RoundedRectangle(cornerRadius: 26, style: .continuous))
        .overlay(RoundedRectangle(cornerRadius: 26, style: .continuous).strokeBorder(Palette.plateEdge, lineWidth: 0.5))
        .shadow(color: .black.opacity(0.18), radius: 24, y: 8)
        .padding(.horizontal, 16)
        .padding(.bottom, 8)
        .accessibilityIdentifier("expiredList")
    }
}

/// A running grant's tile, following its clock: every second in its last two minutes (the seconds show), every 10 s
/// before, and not at all for a grant that never ends by itself.
private struct LiveGrantTile: View {
    var grant: GrantView
    /// Two minutes or less left.
    @State private var close = false

    var body: some View {
        Group {
            if grant.expiresAt == nil {
                GrantTile(grant: grant, now: nowSeconds())
            } else {
                TimelineView(.periodic(from: .now, by: close ? 1 : 10)) { ctx in
                    GrantTile(grant: grant, now: nowSeconds(ctx.date))
                }
            }
        }
        .task(id: grant.expiresAt) {
            guard let end = grant.expiresAt else { return }
            let wait = end - 120 - nowSeconds()
            close = wait <= 0
            guard wait > 0 else { return }
            try? await Task.sleep(for: .seconds(wait))
            if !Task.isCancelled { close = true }
        }
    }
}

/// A grant a sheet is about (`sheet(item:)` needs an identity).
struct GrantPick: Identifiable {
    var grant: GrantView
    var id: String { grant.id }

    init(_ grant: GrantView) { self.grant = grant }
}

extension View {
    /// A list row that draws its own surface: no separator, no background, the given insets.
    func plainRow(_ insets: EdgeInsets) -> some View {
        listRowInsets(insets)
            .listRowSeparator(.hidden)
            .listRowBackground(Color.clear)
    }
}
