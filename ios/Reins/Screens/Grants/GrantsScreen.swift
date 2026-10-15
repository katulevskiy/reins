import SwiftUI

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
                EmptyState(symbol: "clock.arrow.circlepath", title: "Nothing ended")
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
        .navigationTitle("Ended")
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

/// A running grant's card (or, on Trust, its hairline row), following its clock: every second in its last two minutes
/// (the seconds show), every 10 s before, and not at all for a grant that never ends by itself.
struct LiveGrantCard: View {
    var grant: GrantView
    var row = false
    /// Two minutes or less left.
    @State private var close = false

    var body: some View {
        Group {
            if grant.expiresAt == nil {
                GrantCard(grant: grant, now: nowSeconds(), row: row)
            } else {
                TimelineView(.periodic(from: .now, by: close ? 1 : 10)) { ctx in
                    GrantCard(grant: grant, now: nowSeconds(ctx.date), row: row)
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
