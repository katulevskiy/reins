import SwiftUI

// "For others" (decisions v2, 04 §6.2): requests of other accounts that wait for this phone, under "Waiting for you".
// Grouped by person, the soonest due first; each card has the person on top ("Sam's ChatGPT"), what it asks, the
// arrangement's scope line, and Approve (Face ID) or Deny with an optional note. What was settled lately stays,
// dimmed. A watcher's notices follow as a quiet list without buttons.

/// The reviews that wait, by person, then those settled lately: one raised group like "Waiting for you".
struct ForOthers: View {
    var groups: [CircleLogic.ReviewGroup]
    var settled: [CircleReviewView]

    private enum Line: Identifiable {
        case title(Int)
        case who(CircleLogic.ReviewGroup)
        case review(CircleReviewView)
        case settledTitle
        case settled(CircleReviewView)

        var id: String {
            switch self {
            case .title: "title"
            case let .who(g): "who:\(g.id)"
            case let .review(r): "review:\(r.id)"
            case .settledTitle: "settledTitle"
            case let .settled(r): "settled:\(r.id)"
            }
        }

        func hairline(at index: Int) -> (shown: Bool, inset: CGFloat) {
            switch self {
            case .title: (false, 14)
            case .who: (index > 1, 14)
            case .review: (false, 14)
            case .settledTitle: (true, 14)
            case .settled: (false, 14)
            }
        }
    }

    private var lines: [Line] {
        var out: [Line] = [.title(groups.reduce(0) { $0 + $1.reviews.count })]
        for g in groups {
            out.append(.who(g))
            out += g.reviews.map(Line.review)
        }
        if !settled.isEmpty {
            out.append(.settledTitle)
            out += settled.map(Line.settled)
        }
        return out
    }

    var body: some View {
        TimelineView(.periodic(from: .now, by: 30)) { context in
            let now = Int64(context.date.timeIntervalSince1970)
            let lines = self.lines
            VStack(spacing: 0) {
                ForEach(Array(lines.enumerated()), id: \.element.id) { i, line in
                    let rule = line.hairline(at: i)
                    row(line, now: now)
                        .modifier(GroupSlice(first: i == 0, last: i == lines.count - 1, hairline: rule.shown, inset: rule.inset))
                }
            }
        }
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("forOthers")
    }

    @ViewBuilder private func row(_ line: Line, now: Int64) -> some View {
        switch line {
        case let .title(count):
            HStack(spacing: 6) {
                Image(systemName: "person.2").font(.system(size: 12.5, weight: .semibold))
                Text("For others")
                    .font(RFont.sans(13, .semibold))
                    .tracking(13 * TextRole.section.trackingEm)
                    .lineLimit(1)
                Spacer(minLength: 8)
                if count > 0 {
                    Text("\(count)").font(RFont.sans(13, .semibold)).monospacedDigit()
                }
            }
            .foregroundStyle(Palette.ink2)
            .padding(.horizontal, 14)
            .padding(.top, 12)
            .padding(.bottom, 4)
            .frame(minHeight: 40)
            .accessibilityElement(children: .ignore)
            .accessibilityLabel("For others, \(count)")
            .accessibilityAddTraits(.isHeader)
        case let .who(group):
            HStack(spacing: 8) {
                BlobAvatar(seed: untrusted(group.person), size: 24)
                Text(verbatim: "Waiting for \(CircleLogic.name(group.person))")
                    .font(RFont.sans(14, .semibold))
                    .foregroundStyle(Palette.ink)
                    .lineLimit(1)
                Spacer(minLength: 6)
                DueText(deadline: group.deadline, now: now)
            }
            .padding(.horizontal, 14)
            .padding(.top, 8)
            .padding(.bottom, 2)
            .accessibilityElement(children: .combine)
            .accessibilityAddTraits(.isHeader)
            .accessibilityIdentifier("forOthersWho:\(group.person)")
        case let .review(review):
            CircleReviewCard(review: review, now: now)
        case .settledTitle:
            Text("Settled")
                .font(RFont.sans(12.5, .semibold))
                .foregroundStyle(Palette.inkMeta)
                .padding(.horizontal, 14)
                .padding(.top, 10)
                .padding(.bottom, 2)
                .accessibilityAddTraits(.isHeader)
        case let .settled(review):
            SettledReviewRow(review: review)
        }
    }
}

/// One request of another account: whose agent asks, what, the details and the arrangement's scope, when it is due;
/// Approve (with the phone's check) and Deny (with an optional note).
private struct CircleReviewCard: View {
    var review: CircleReviewView
    var now: Int64
    @Environment(AppModel.self) private var model
    @Environment(\.feedback) private var feedback
    @State private var busy = false
    @State private var denying = false
    @State private var note = ""
    @State private var error: String?

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            HStack(spacing: 6) {
                Text(verbatim: CircleLogic.whose(review))
                    .font(RFont.sans(13, .semibold))
                    .foregroundStyle(Palette.ink2)
                    .lineLimit(1)
                CircleRoleChip(role: review.role)
                Spacer(minLength: 0)
            }
            Text(verbatim: untrusted(review.title))
                .font(RFont.sans(TextRole.body.size, .semibold))
                .foregroundStyle(Palette.ink)
                .lineLimit(3)
                .fixedSize(horizontal: false, vertical: true)
            if let sub = nonEmpty(untrusted(review.subtitle)) {
                Text(verbatim: sub)
                    .font(RFont.mono(12.5))
                    .foregroundStyle(Palette.ink2)
                    .lineLimit(2)
                    .environment(\.layoutDirection, .leftToRight)
            }
            if let detail = review.detail.map(untrusted).flatMap(nonEmpty) {
                Text(verbatim: detail)
                    .font(RFont.sans(13))
                    .foregroundStyle(Palette.ink2)
                    .lineLimit(4)
            }
            HStack(spacing: 6) {
                if let line = nonEmpty(untrusted(review.line)) {
                    Text(verbatim: line).lineLimit(1)
                }
                if let scope = nonEmpty(untrusted(review.scopeLine)) {
                    Text(verbatim: "· \(scope)").lineLimit(1)
                }
            }
            .font(RFont.sans(12.5))
            .foregroundStyle(Palette.inkMeta)
            if let error {
                Text(error).font(RFont.sans(13, .medium)).foregroundStyle(Palette.danger).fixedSize(horizontal: false, vertical: true)
            }
            HStack(spacing: 8) {
                Button {
                    answer(approve: true)
                } label: {
                    HStack(spacing: 6) {
                        if busy { ProgressView().tint(Palette.onInverse) } else { BiometryMark(size: 15) }
                        Text("Approve")
                    }
                }
                .buttonStyle(SignalButtonStyle(.inverse, size: .small, fill: false))
                .accessibilityIdentifier("circleApprove:\(review.id)")
                Button("Deny") {
                    note = ""
                    denying = true
                    feedback.play(.open)
                }
                .buttonStyle(SignalButtonStyle(.outline, size: .small, fill: false))
                .accessibilityIdentifier("circleDeny:\(review.id)")
                Spacer(minLength: 0)
            }
            .disabled(busy)
            .padding(.top, 4)
        }
        .padding(.horizontal, 14)
        .padding(.vertical, 10)
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("circleReview:\(review.id)")
        .alert("Deny for \(CircleLogic.name(review.person))?", isPresented: $denying) {
            TextField("Note (optional)", text: $note)
                .accessibilityIdentifier("circleDenyNote")
            Button("Deny", role: .destructive) { answer(approve: false, note: note) }
                .accessibilityIdentifier("circleDenyConfirm")
            Button("Cancel", role: .cancel) {}
        }
    }

    private func nonEmpty(_ s: String) -> String? { s.isEmpty ? nil : s }

    private func answer(approve: Bool, note: String? = nil) {
        guard !busy else { return }
        busy = true
        error = nil
        Task {
            error = await model.answerCircle(review, approve: approve, note: note)
            busy = false
        }
    }
}

/// A review settled lately, dimmed: whose, what, and how it ended.
private struct SettledReviewRow: View {
    var review: CircleReviewView

    var body: some View {
        HStack(spacing: 10) {
            VStack(alignment: .leading, spacing: 2) {
                Text(verbatim: CircleLogic.whose(review))
                    .font(RFont.sans(12.5, .semibold))
                    .foregroundStyle(Palette.inkMeta)
                    .lineLimit(1)
                Text(verbatim: untrusted(review.title))
                    .font(RFont.sans(14))
                    .foregroundStyle(Palette.ink2)
                    .lineLimit(1)
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            Pill(CircleLogic.outcome(review.state), tone: review.state == .approved ? .success : review.state == .denied ? .danger : .neutral)
        }
        .padding(.horizontal, 14)
        .padding(.vertical, 8)
        .opacity(0.6)
        .accessibilityElement(children: .combine)
        .accessibilityIdentifier("circleSettled:\(review.id)")
    }
}

/// What a watcher was told: a quiet list, no buttons ("Dana · Turned on Bypass for 1 h"). A ward's weekly summary
/// ("This week: 3 automatic decisions") keeps its lines behind More.
struct CircleNotices: View {
    var notices: [CircleNoticeView]
    @State private var open: Set<String> = []

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            SectionHeader("Told", glyph: .eye, quiet: true)
            ForEach(notices, id: \.id) { n in
                HStack(alignment: .firstTextBaseline, spacing: 10) {
                    Image(systemName: "eye").font(.system(size: 12, weight: .semibold)).foregroundStyle(Palette.inkMeta)
                    VStack(alignment: .leading, spacing: 2) {
                        Text(verbatim: "\(CircleLogic.name(n.person)) · \(untrusted(n.text))")
                            .font(RFont.sans(14))
                            .foregroundStyle(Palette.ink2)
                            .fixedSize(horizontal: false, vertical: true)
                        if let outcome = n.outcome.map(untrusted), !outcome.isEmpty {
                            Text(verbatim: outcome).font(RFont.sans(12.5)).foregroundStyle(Palette.inkMeta)
                        }
                        if !n.lines.isEmpty {
                            if open.contains(n.id) {
                                ForEach(Array(n.lines.prefix(50).enumerated()), id: \.offset) { _, line in
                                    Text(verbatim: untrusted(line)).font(RFont.sans(12.5)).foregroundStyle(Palette.inkMeta)
                                }
                            }
                            Button(open.contains(n.id) ? "Less" : "More") {
                                if open.contains(n.id) { open.remove(n.id) } else { open.insert(n.id) }
                            }
                            .font(RFont.sans(12.5, .semibold))
                            .foregroundStyle(Palette.violet)
                            .buttonStyle(.plain)
                            .accessibilityIdentifier("circleNoticeMore:\(n.id)")
                        }
                    }
                    .frame(maxWidth: .infinity, alignment: .leading)
                    Text(TimeText.relative(n.at)).font(RFont.sans(12.5)).foregroundStyle(Palette.inkMeta)
                }
                .padding(.horizontal, 24)
                .padding(.vertical, 6)
                .accessibilityElement(children: .combine)
                .accessibilityIdentifier("circleNotice:\(n.id)")
            }
        }
        .accessibilityIdentifier("circleNotices")
    }
}
