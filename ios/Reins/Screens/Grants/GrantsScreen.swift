import SwiftUI

/// The grants that ended (the Trust board, 05; quiet): grouped by how they ended (expired, used up, ended by you), each
/// can be resumed, or deleted for good.
struct ExpiredGrantsScreen: View {
    @Environment(AppModel.self) private var model
    @Environment(\.feedback) private var feedback
    @State private var resuming: GrantPick?
    @State private var deleting: GrantView?
    @State private var error: String?

    private var ended: [GrantView] { model.grants.filter { !$0.active }.sorted { $0.createdAt > $1.createdAt } }

    var body: some View {
        let groups = EndedGroup.split(ended)
        List {
            Text("Ended")
                .textRole(.quietTitle)
                .foregroundStyle(Palette.ink)
                .accessibilityAddTraits(.isHeader)
                .plainRow(EdgeInsets(top: 4, leading: 20, bottom: 0, trailing: 20))
            if let error {
                FormBanner(text: error)
                    .accessibilityIdentifier("resumeError")
                    .plainRow(EdgeInsets(top: 10, leading: 20, bottom: 0, trailing: 20))
            }
            if ended.isEmpty {
                EmptyState.calm("Nothing ended.")
                    .padding(.top, 40)
                    .plainRow(EdgeInsets())
            }
            ForEach(groups, id: \.kind) { group in
                SectionHeader(group.kind.title, glyph: group.kind.glyph, quiet: true, trailing: "\(group.grants.count)")
                    .plainRow(EdgeInsets())
                FlatGroup(inset: 0) {
                    ForEach(group.grants, id: \.id) { grant in
                        EndedGrantRow(
                            grant: grant,
                            onOpen: {
                                model.push(.grantDetail(grant.id))
                                feedback.defaultTap()
                            },
                            onResume: { resuming = GrantPick(grant) },
                            onDelete: { deleting = grant }
                        )
                        .contextMenu {
                            Button { tapped { model.push(.grantDetail(grant.id)) } } label: { Label("Open", systemImage: "key.horizontal") }
                            Button { tapped { resuming = GrantPick(grant) } } label: { Label("Resume", systemImage: "arrow.clockwise") }
                            Button(role: .destructive) { tapped { deleting = grant } } label: { Label("Delete for good", systemImage: "trash") }
                        }
                    }
                }
                .plainRow(EdgeInsets())
            }
        }
        .listStyle(.plain)
        .scrollContentBackground(.hidden)
        .contentMargins(.bottom, 12, for: .scrollContent)
        .accessibilityIdentifier("expiredList")
        .pageBackground()
        .navigationTitle("")
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

/// How the ended grants group: expired, used up, ended by you, in that order; empty groups are left out.
struct EndedGroup {
    enum Kind: CaseIterable {
        case expired, usedUp, byYou

        var title: String {
            switch self {
            case .expired: "Expired"
            case .usedUp: "Used up"
            case .byYou: "Ended by you"
            }
        }

        var glyph: Glyph {
            switch self {
            case .expired: .timer
            case .usedUp: .card
            case .byYou: .timerOff
            }
        }

        static func of(_ g: GrantView) -> Kind {
            switch g.state {
            case "revoked": .byYou
            case "used_up": .usedUp
            default: .expired
            }
        }
    }

    var kind: Kind
    var grants: [GrantView]

    static func split(_ ended: [GrantView]) -> [EndedGroup] {
        Kind.allCases.compactMap { kind in
            let grants = ended.filter { Kind.of($0) == kind }
            return grants.isEmpty ? nil : EndedGroup(kind: kind, grants: grants)
        }
    }

    /// The line under an ended grant: who it was for, and when or how it ended ("Claude · 2 days ago", "notes-bot · 3 of
    /// 3 uses").
    static func line(_ g: GrantView, now: Int64 = nowSeconds()) -> String {
        let who = untrustedName(g.connectionLabel)
        let why: String? = switch Kind.of(g) {
        case .expired: g.expiresAt.map { GrantText.relative($0, now: now) }
        case .usedUp: g.maxUses.map { "\(g.uses) of \($0) uses" }
        case .byYou: nil
        }
        return [who, why].compactMap { $0 }.filter { !$0.isEmpty }.joined(separator: " · ")
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
