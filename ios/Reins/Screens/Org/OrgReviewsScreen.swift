import SwiftUI

/// Settings > Reviews (a work profile's): what waits for this member's review in their org. Each card shows the org,
/// who must approve, the title and the lines the phone built from what it signs; Approve and Deny sign with the key
/// this phone enrolled to the org. Your own admin change is confirmed instead.
struct OrgReviewsScreen: View {
    @Environment(AppModel.self) private var model
    @Environment(\.feedback) private var feedback
    @State private var vm: OrgReviewsModel?

    var body: some View {
        QuietPage("Reviews") {
            if let vm {
                VStack(alignment: .leading, spacing: 12) {
                    if let error = vm.error { QuietBanner(text: error).accessibilityIdentifier("orgReviewsError") }
                    if let reviews = vm.reviews {
                        if reviews.isEmpty {
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
