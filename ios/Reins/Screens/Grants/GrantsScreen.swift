import SwiftUI

/// The Grants section: the permissions that are running, each a card with its time left; the ones that ended folded
/// into one "Expired" row; and what a newly connected AI may do. Swiping a running grant ends it (it can be resumed
/// later from Expired).
struct GrantsScreen: View {
    @Environment(AppModel.self) private var model
    @Environment(\.feedback) private var feedback
    @State private var revoking: GrantView?
    @State private var error: String?

    private var running: [GrantView] { model.grants.filter(\.active).sorted { $0.createdAt > $1.createdAt } }
    private var endedCount: Int { model.grants.filter { !$0.active }.count }

    var body: some View {
        let running = running
        let endedCount = endedCount
        List {
            PageHeader(title: "Grants") {
                RoundPlusButton(label: "New grant") { model.show(.newGrant(token: 0)) }
                    .accessibilityIdentifier("newGrant")
            }
            .padding(.top, 8)
            .plainRow(EdgeInsets(top: 0, leading: 20, bottom: 0, trailing: 16))

            if let error {
                FormBanner(text: error)
                    .accessibilityIdentifier("resumeError")
                    .plainRow(EdgeInsets(top: 6, leading: 16, bottom: 6, trailing: 16))
            }

            if running.isEmpty {
                EmptyState(symbol: "key.horizontal", title: "No active grants")
                    .padding(.top, 24)
                    .accessibilityIdentifier("noGrants")
                    .plainRow(EdgeInsets())
            } else {
                PageSectionHeader("Active · \(running.count)")
                    .plainRow(EdgeInsets(top: 0, leading: 0, bottom: 0, trailing: 0))
            }

            ForEach(running, id: \.id) { grant in
                Button {
                    model.show(.grantDetail(grant.id))
                } label: {
                    LiveGrantCard(grant: grant)
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
                .plainRow(EdgeInsets(top: 5, leading: 12, bottom: 5, trailing: 12))
            }

            if endedCount > 0 {
                expiredRow(endedCount)
                    .padding(.top, 4)
                    .plainRow(EdgeInsets(top: 5, leading: 16, bottom: 5, trailing: 16))
            }

            // The starting rule for new AIs (the card brings its own side margin).
            StartingRuleChooser()
                .padding(.top, 6)
                .plainRow(EdgeInsets(top: 6, leading: 0, bottom: 6, trailing: 0))
        }
        .listStyle(.plain)
        .scrollContentBackground(.hidden)
        .contentMargins(.bottom, 24, for: .scrollContent)
        .refreshable {
            await model.refreshPending()
            feedback.play(.refresh)
        }
        .pageBackground()
        .navigationTitle("Grants")
        .rootNavigationBar()
        .confirmationDialog(
            "End this grant now?",
            isPresented: Binding(get: { revoking != nil }, set: { if !$0 { revoking = nil } }),
            titleVisibility: .visible,
            presenting: revoking
        ) { grant in
            Button("End now", role: .destructive) {
                error = nil
                feedback.quietClose()
                Task { error = await model.revokeGrant(grant.id) }
            }
        } message: { grant in
            Text(untrusted(grant.summary) + "\n\nYou can resume it later from Expired.")
        }
        .presentationFeedback(revoking != nil)
    }

    /// A menu or swipe item: its action, then the default tap (the page or dialog it opens has the sound).
    private func tapped(_ action: () -> Void) {
        action()
        feedback.defaultTap()
    }

    /// "Expired  4 ›": the grants that ended, one tap away.
    private func expiredRow(_ count: Int) -> some View {
        Button {
            model.show(.expiredGrants)
        } label: {
            HStack(spacing: 13) {
                Image(systemName: "clock.arrow.circlepath")
                    .font(.system(size: 15, weight: .semibold))
                    .foregroundStyle(Palette.secondary)
                    .frame(width: 32, height: 32)
                    .background(Palette.controlFill, in: RoundedRectangle(cornerRadius: 9, style: .continuous))
                Text("Expired")
                    .font(RFont.sans(16, .medium))
                    .foregroundStyle(Palette.text)
                Spacer(minLength: 8)
                Text("\(count)")
                    .font(RFont.sans(15))
                    .foregroundStyle(Palette.secondary)
                Image(systemName: "chevron.right")
                    .font(.system(size: 13, weight: .semibold))
                    .foregroundStyle(Palette.tertiary)
            }
            .padding(.horizontal, 14)
            .frame(minHeight: 56)
            .background(Palette.elevated, in: RoundedRectangle(cornerRadius: 20, style: .continuous))
            .overlay(RoundedRectangle(cornerRadius: 20, style: .continuous).strokeBorder(Palette.hairline, lineWidth: 1))
            .contentShape(RoundedRectangle(cornerRadius: 20, style: .continuous))
        }
        .buttonStyle(.plain)
        .feedbackTap(.tap, feedback)
        .accessibilityElement(children: .combine)
        .accessibilityLabel("Expired, \(count)")
        .accessibilityIdentifier("expiredHeader")
    }
}

/// The grants that ended (expired, used up or deleted): each can be resumed, or deleted for good.
struct ExpiredGrantsScreen: View {
    @Environment(AppModel.self) private var model
    @Environment(\.feedback) private var feedback
    @State private var resuming: GrantPick?
    @State private var deleting: GrantView?
    @State private var error: String?

    private var ended: [GrantView] { model.grants.filter { !$0.active }.sorted { $0.createdAt > $1.createdAt } }

    var body: some View {
        let ended = ended
        List {
            if let error {
                FormBanner(text: error)
                    .accessibilityIdentifier("resumeError")
                    .plainRow(EdgeInsets(top: 6, leading: 16, bottom: 6, trailing: 16))
            }
            if ended.isEmpty {
                EmptyState(symbol: "clock.arrow.circlepath", title: "Nothing expired")
                    .padding(.top, 40)
                    .plainRow(EdgeInsets())
            }
            ForEach(ended, id: \.id) { grant in
                EndedGrantRow(
                    grant: grant,
                    onOpen: {
                        model.push(.grantDetail(grant.id))
                        feedback.defaultTap()
                    },
                    onResume: { resuming = GrantPick(grant) },
                    onDelete: { deleting = grant }
                )
                .background(Palette.elevated, in: RoundedRectangle(cornerRadius: 20, style: .continuous))
                .contextMenu {
                    Button { tapped { model.push(.grantDetail(grant.id)) } } label: { Label("Open", systemImage: "key.horizontal") }
                    Button { tapped { resuming = GrantPick(grant) } } label: { Label("Resume", systemImage: "arrow.clockwise") }
                    Button(role: .destructive) { tapped { deleting = grant } } label: { Label("Delete for good", systemImage: "trash") }
                }
                .plainRow(EdgeInsets(top: 5, leading: 12, bottom: 5, trailing: 12))
            }
        }
        .listStyle(.plain)
        .scrollContentBackground(.hidden)
        .contentMargins(.vertical, 12, for: .scrollContent)
        .accessibilityIdentifier("expiredList")
        .pageBackground()
        .navigationTitle("Expired")
        .navigationBarTitleDisplayMode(.inline)
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
    }

    private func tapped(_ action: () -> Void) {
        action()
        feedback.defaultTap()
    }
}

/// A running grant's card, following its clock: every second in its last two minutes (the seconds show), every 10 s
/// before, and not at all for a grant that never ends by itself.
private struct LiveGrantCard: View {
    var grant: GrantView
    /// Two minutes or less left.
    @State private var close = false

    var body: some View {
        Group {
            if grant.expiresAt == nil {
                GrantCard(grant: grant, now: nowSeconds())
            } else {
                TimelineView(.periodic(from: .now, by: close ? 1 : 10)) { ctx in
                    GrantCard(grant: grant, now: nowSeconds(ctx.date))
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
