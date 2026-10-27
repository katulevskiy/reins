import SwiftUI

// "Auto needs information" (01 §3, later questions): a party a request named that Auto doesn't know, as a card with the
// waiting ones, in the same shell. "Who is octo on GitHub?", which AI's request named it, then the four zones; the
// xmark puts it away (it stays Outside and isn't asked again). Anything but Outside is the owner's to say: the core asks
// for the check. Listed only, never notified: nothing waits on the answer.

/// The question cards in words (pure, unit-tested).
enum QuestionText {
    /// At most this many characters of what it is about.
    static let limit = 40

    /// "Who is octo on GitHub?" for a GitHub user or organization, "Who is bob@acme.io?" for anything else.
    static func title(_ q: AutoQuestion) -> String { runs(q).map(\.text).joined() }

    /// The title as an ask, the thing itself as its target.
    static func runs(_ q: AutoQuestion) -> [AskText.Run] {
        let key = untrustedName(q.key, limit: limit)
        return [AskText.Run(text: "Who is "), AskText.Run(text: key, target: true), AskText.Run(text: q.kind == .owner ? " on GitHub?" : "?")]
    }

    /// The quiet line under it: which AI's request named it.
    static func from(_ q: AutoQuestion) -> String? {
        let name = untrustedName(q.connectionLabel, limit: WaitCardIdentity.limit)
        return name.isEmpty ? nil : "“\(name)”"
    }

    /// The card's field: Auto's violet.
    static let field = FieldKind.grant
}

/// The open questions under what waits: the newest as a card, "+N" opens the rest one under another.
struct QuestionCards: View {
    var questions: [AutoQuestion]
    @State private var expanded = false
    @Environment(\.feedback) private var feedback
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    var body: some View {
        VStack(spacing: 10) {
            ForEach(expanded ? questions : Array(questions.prefix(1)), id: \.id) { q in
                QuestionCard(question: q)
                    .transition(reduceMotion ? .opacity : .asymmetric(insertion: .move(edge: .top).combined(with: .opacity), removal: .opacity))
            }
            if questions.count > 1 {
                Button {
                    withAnimation(Motion.of(Motion.easeOut(Motion.fold), reduce: reduceMotion)) { expanded.toggle() }
                    feedback.play(.expand(expanded))
                } label: {
                    Group {
                        if expanded {
                            Image(systemName: "chevron.up").font(.system(size: 14, weight: .semibold))
                        } else {
                            Text("+\(questions.count - 1)").font(RFont.sans(12, .bold)).monospacedDigit()
                        }
                    }
                    .foregroundStyle(Palette.onInverse)
                    .padding(.horizontal, 9)
                    .frame(minWidth: 26, minHeight: 22)
                    .background(Palette.inverse, in: Capsule())
                    .frame(minWidth: 44, minHeight: 44)
                    .contentShape(Rectangle())
                }
                .buttonStyle(PressDimsStyle())
                .accessibilityLabel(expanded ? "Show one" : "\(questions.count - 1) more questions")
                .accessibilityIdentifier("questionsToggle")
            }
        }
        .padding(.horizontal, 16)
        .animation(Motion.of(Motion.cardSpring(0.32), reduce: reduceMotion), value: questions.map(\.id))
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("questions")
    }
}

/// One question: the service's symbol, the xmark; "Who is octo on GitHub?"; who asked, quietly; Mine · Friends · Work ·
/// Outside, the proposed one in white.
struct QuestionCard: View {
    var question: AutoQuestion
    @Environment(AppModel.self) private var model
    @State private var busy = false
    @State private var error: String?

    var body: some View {
        WaitCardShell(
            kind: QuestionText.field, label: "Auto needs information",
            ask: AskText(QuestionText.runs(question), role: .cardAsk), askIdentifier: "questionTitle:\(question.id)"
        ) {
            Image(systemName: AutoSetupText.serviceSymbol(question.service))
                .font(.system(size: 16, weight: .semibold))
                .foregroundStyle(Palette.onField)
                .frame(width: 34, height: 34)
                .background(Palette.onFieldWash, in: Circle())
        } timer: {
            Button {
                Task { await dismiss() }
            } label: {
                Image(systemName: "xmark")
                    .font(.system(size: 14, weight: .semibold))
                    .foregroundStyle(Palette.onField)
                    .frame(width: 32, height: 32)
                    .background(Palette.onFieldWash, in: Circle())
                    .frame(width: 44, height: 44)
                    .contentShape(Rectangle())
            }
            .buttonStyle(PressDimsStyle())
            .accessibilityLabel("Dismiss")
            .accessibilityIdentifier("dismissQuestion:\(question.id)")
        } actions: {
            VStack(alignment: .leading, spacing: 12) {
                if let from = QuestionText.from(question) {
                    Text(verbatim: from)
                        .font(RFont.sans(13.5, .medium))
                        .foregroundStyle(Palette.onField2)
                        .lineLimit(1)
                        .padding(.top, -4)
                        .accessibilityLabel("From \(from)")
                }
                if let error {
                    Text(error).font(RFont.sans(13.5, .medium)).foregroundStyle(Palette.onField).lineLimit(3)
                        .accessibilityIdentifier("questionError")
                }
                ViewThatFits(in: .horizontal) {
                    HStack(spacing: 6) { zoneButtons }
                    VStack(spacing: 6) {
                        HStack(spacing: 6) { zoneButtons(AutoSetupText.zones.prefix(2)) }
                        HStack(spacing: 6) { zoneButtons(AutoSetupText.zones.suffix(2)) }
                    }
                }
            }
        }
        .disabled(busy)
        .accessibilityIdentifier("question:\(question.id)")
    }

    @ViewBuilder private var zoneButtons: some View { zoneButtons(AutoSetupText.zones[...]) }

    private func zoneButtons(_ zones: ArraySlice<ZoneChoice>) -> some View {
        ForEach(Array(zones), id: \.self) { zone in
            Button(AutoSetupText.zoneName(zone)) { Task { await answer(zone) } }
                .buttonStyle(SignalButtonStyle(zone == question.proposed ? .onField : .onFieldWash, size: .small))
                .accessibilityIdentifier("answer:\(question.id):\(AutoSetupText.zoneId(zone))")
        }
    }

    private func answer(_ zone: ZoneChoice) async {
        busy = true
        error = await model.answerAutoQuestion(question.id, zone: zone)
        busy = false
    }

    private func dismiss() async {
        busy = true
        error = await model.dismissAutoQuestion(question.id)
        busy = false
    }
}
