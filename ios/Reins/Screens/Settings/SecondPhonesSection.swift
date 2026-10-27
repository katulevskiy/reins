import Observation
import SwiftUI

/// Settings › Devices, on the approving phone of an account with phone roles: the second phones (viewers) it added,
/// each removable with a hold. Shows nothing on a legacy account or on a viewer.
struct SecondPhonesSection: View {
    @Environment(AppModel.self) private var model
    @Environment(\.feedback) private var feedback
    @State private var vm = SecondPhonesModel()
    @State private var removing: RosterMemberView?

    var body: some View {
        Group {
            if let roster = vm.roster, roster.role == .primary {
                let viewers = SecondPhonesModel.viewers(of: roster)
                QuietHelpHeader(title: "Second phones", help: "They see your connections and activity. They decide nothing.")
                FlatGroup {
                    if viewers.isEmpty {
                        TileRow("None", glyph: .phone, chevron: false)
                            .flatRow()
                            .accessibilityLabel("No second phone.")
                            .accessibilityIdentifier("noViewers")
                    }
                    ForEach(viewers, id: \.deviceId) { member in
                        TileRow(untrustedName(member.name, limit: 40), chevron: false) {
                            GlyphTile(.phone)
                        } trailing: {
                            Button("Remove", role: .destructive) { removing = member }
                                .buttonStyle(.borderless)
                                .font(RFont.sans(15, .semibold))
                                .foregroundStyle(Palette.danger)
                                .frame(minHeight: 44)
                                .disabled(vm.busy)
                                .accessibilityLabel("Remove \(untrustedName(member.name, limit: 40))")
                                .accessibilityIdentifier("removeViewer:\(member.deviceId)")
                        }
                        .flatRow()
                        .accessibilityElement(children: .contain)
                        .accessibilityIdentifier("viewer:\(member.deviceId)")
                    }
                }
                if let error = vm.error { QuietBanner(text: error).accessibilityIdentifier("viewersError") }
            }
        }
        .task { await vm.load(model) }
        .sheet(item: Binding(get: { removing.map(RemovingViewer.init) }, set: { if $0 == nil { removing = nil } })) { item in
            RemoveViewerSheet(member: item.member, busy: vm.busy) {
                Task {
                    if await vm.remove(item.member, model) { removing = nil }
                }
            } onCancel: { removing = nil }
            .environment(\.feedback, feedback)
        }
        .presentationFeedback(removing != nil)
    }
}

private struct RemovingViewer: Identifiable {
    var member: RosterMemberView
    var id: String { member.deviceId }
}

@Observable
@MainActor
final class SecondPhonesModel {
    private(set) var roster: RosterSummary?
    private(set) var busy = false
    private(set) var error: String?

    /// The phones this one added: never the primary, never this phone.
    static func viewers(of roster: RosterSummary) -> [RosterMemberView] {
        roster.members.filter { !$0.primary && !$0.thisPhone }
    }

    func load(_ app: AppModel) async {
        roster = try? await app.core.rosterSummary(refresh: true)
    }

    /// Takes a second phone off the account: it reads nothing written from now on.
    func remove(_ member: RosterMemberView, _ app: AppModel) async -> Bool {
        guard !busy, !member.primary, !member.thisPhone else { return false }
        busy = true
        error = nil
        defer { busy = false }
        do {
            try await app.core.rosterRemoveMember(deviceId: member.deviceId)
            app.feedback.play(.revoked)
            app.notice = "\(PairWords.quoted(untrustedName(member.name, limit: 40))) no longer sees this account."
            roster = try? await app.core.rosterSummary(refresh: false)
            return true
        } catch {
            app.feedback.play(.error)
            self.error = error.userMessage
            return false
        }
    }
}

/// "Remove a second phone": which one, what removing does, and Remove held to confirm.
private struct RemoveViewerSheet: View {
    var member: RosterMemberView
    var busy: Bool
    var onConfirm: () -> Void
    var onCancel: () -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            Text("Remove a second phone")
                .font(RFont.sans(22, .semibold))
                .foregroundStyle(Palette.ink)
            HStack(spacing: 12) {
                GlyphIcon(.phone, size: 20).foregroundStyle(Palette.ink2)
                Text(verbatim: PairWords.quoted(untrustedName(member.name, limit: 40)))
                    .font(RFont.sans(16, .medium))
                    .foregroundStyle(Palette.ink)
                    .lineLimit(1)
            }
            Text("It no longer sees this account.")
                .font(RFont.sans(15))
                .foregroundStyle(Palette.ink2)
            RatioRow(spacing: 10, ratio: 1) {
                Button("Cancel", action: onCancel)
                    .buttonStyle(SignalButtonLook(kind: .outline))
                HoldButton(title: "Remove", busy: busy, onConfirm: onConfirm)
                    .accessibilityIdentifier("confirmRemoveViewer")
            }
            .frame(height: 54)
        }
        .padding(24)
        .presentationDetents([.medium])
        .presentationCornerRadius(32)
        .accessibilityIdentifier("removeViewerSheet")
    }
}
