import SwiftUI

/// Settings > Reviews (a work profile's): what waits for this member's review in their org. Each card shows the org,
/// who must approve, the title and the lines the phone built from what it signs, then who else must still decide and
/// where an escalation took it; Approve and Deny sign with the key this phone enrolled to the org. Your own admin change
/// is confirmed instead. A web passkey added in the browser waits on top until this phone confirms it.
struct OrgReviewsScreen: View {
    @Environment(AppModel.self) private var model
    @Environment(\.feedback) private var feedback
    @State private var vm: OrgReviewsModel?

    var body: some View {
        QuietPage("Reviews") {
            if let vm {
                VStack(alignment: .leading, spacing: 12) {
                    if let error = vm.error { QuietBanner(text: error).accessibilityIdentifier("orgReviewsError") }
                    ForEach(vm.passkeys, id: \.credentialId) { key in
                        WebPasskeyCard(key: key, busy: vm.busy == key.credentialId, enabled: vm.busy == nil) { approve in
                            Task {
                                await vm.confirmPasskey(key, approve: approve)
                                feedback.play(.refresh)
                            }
                        }
                    }
                    if let reviews = vm.reviews {
                        if reviews.isEmpty && vm.passkeys.isEmpty {
                            EmptyState(symbol: "checkmark.shield", title: "No reviews")
                                .frame(maxWidth: .infinity)
                                .accessibilityIdentifier("noOrgReviews")
                        }
                        ForEach(reviews, id: \.id) { review in
                            OrgReviewCard(review: review, busy: vm.busy == review.id, enabled: vm.busy == nil) { approve in
                                Task {
                                    await vm.answer(review, approve: approve)
                                    feedback.play(.refresh)
                                }
                            }
                        }
                    }
                }
                .padding(16)
            }
        }
        .task {
            let m = vm ?? OrgReviewsModel(core: CoreOrgReviews(core: model.core))
            vm = m
            await m.load()
        }
        .refreshable { await vm?.load() }
    }
}

struct OrgReviewCard: View {
    let review: OrgReviewView
    var busy = false
    var enabled = true
    let answer: (Bool) -> Void

    private var confirm: Bool { review.own && review.kind == .change }

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            HStack(spacing: 6) {
                if review.urgent {
                    Pill("Urgent", tone: .danger, glyph: .warning).accessibilityIdentifier("orgUrgent:\(review.id)")
                }
                Pill(untrustedName(review.orgName, limit: 40), tone: .violet, glyph: .briefcase)
                Pill("requires \(untrustedName(review.requires, limit: 40))", tone: .neutral, glyph: .shieldCheck)
            }
            Text(untrustedName(review.title, limit: 160))
                .font(RFont.sans(16.5, .semibold))
                .foregroundStyle(Palette.ink)
            let who = [review.requester, review.connection].filter { !$0.isEmpty }.joined(separator: " · ")
            if !who.isEmpty {
                Text(untrustedName(who, limit: 120)).font(RFont.sans(13.5)).foregroundStyle(Palette.inkMeta).lineLimit(1)
            }
            ForEach(Array(review.lines.enumerated()), id: \.offset) { _, line in
                Text(untrustedName(line, limit: 160)).font(RFont.sans(14.5)).foregroundStyle(Palette.ink)
            }
            if let reason = OrgRouting.reason(review) {
                RouteLine(symbol: "text.bubble", text: reason, color: Palette.inkMeta, lines: 3)
                    .accessibilityIdentifier("orgReason:\(review.id)")
            }
            if let needs = OrgRouting.needsLine(review) {
                RouteLine(symbol: "person.2", text: needs, color: Palette.ink2)
                    .accessibilityIdentifier("orgNeeds:\(review.id)")
            }
            if let step = OrgRouting.stepLine(review) {
                RouteLine(symbol: "arrow.up.forward.circle", text: step, color: Palette.warning)
                    .accessibilityIdentifier("orgStep:\(review.id)")
            }
            HStack(spacing: 10) {
                Button("Deny") { answer(false) }
                    .buttonStyle(SignalButtonStyle(.danger, size: .small))
                    .disabled(!enabled)
                    .accessibilityIdentifier("orgDeny:\(review.id)")
                Button(confirm ? "Confirm" : "Approve") { answer(true) }
                    .buttonStyle(SignalButtonStyle(.inverse, size: .small))
                    .disabled(!enabled || busy)
                    .accessibilityIdentifier(confirm ? "orgConfirm:\(review.id)" : "orgApprove:\(review.id)")
            }
            .padding(.top, 4)
        }
        .padding(14)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(RoundedRectangle(cornerRadius: 18, style: .continuous).fill(Palette.paperRaised))
        .accessibilityIdentifier("orgReview:\(review.id)")
    }
}

/// A review's routing in a few words, from what the server says is still waited for.
enum OrgRouting {
    /// "Needs On-call: payments · 1 of 2"; nil when one approval decides and nothing else waits.
    static func needsLine(_ review: OrgReviewView) -> String? {
        let needs = review.needs.map { untrustedName($0, limit: 60) }.filter { !$0.isEmpty }
        let progress = untrustedName(review.progress, limit: 20)
        var parts: [String] = []
        if !needs.isEmpty { parts.append("Needs " + needs.joined(separator: ", ")) }
        if !progress.isEmpty { parts.append(progress) }
        return parts.isEmpty ? nil : parts.joined(separator: " · ")
    }

    /// "Escalated to Security admin", when an escalation took it there.
    static func stepLine(_ review: OrgReviewView) -> String? {
        guard let step = review.step.map({ untrustedName($0, limit: 80) }), !step.isEmpty else { return nil }
        return step
    }

    /// The agent's reason, as it gave it.
    static func reason(_ review: OrgReviewView) -> String? {
        guard let reason = review.reason.map({ untrustedName($0, limit: 160) }), !reason.isEmpty else { return nil }
        return reason
    }
}

private struct RouteLine: View {
    let symbol: String
    let text: String
    let color: Color
    var lines = 2

    var body: some View {
        HStack(alignment: .firstTextBaseline, spacing: 6) {
            Image(systemName: symbol).font(.system(size: 12, weight: .semibold)).accessibilityHidden(true)
            Text(text).font(RFont.sans(13.5)).lineLimit(lines)
        }
        .foregroundStyle(color)
        .accessibilityElement(children: .combine)
    }
}
