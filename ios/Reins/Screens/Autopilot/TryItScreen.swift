import SwiftUI

/// "Try it": a request typed in the situation format the model reads (spec §4), and what a profile would do with it:
/// the verdict, how sure, the decisions it is like, and why. Nothing is kept.
struct TryItScreen: View {
    var profileId: String?

    @Environment(AppModel.self) private var model
    @Environment(\.feedback) private var feedback
    @State private var ap = AutopilotModel()
    @State private var text = AutopilotText.examples[0].situation
    @State private var example: String? = AutopilotText.examples[0].title
    @State private var chosenProfile: String?
    @FocusState private var editing: Bool

    private var profile: ProfileView? {
        let id = chosenProfile ?? profileId
        return ap.profiles.first { $0.id == id } ?? ap.profiles.first(where: \.isDefault)
    }

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 0) {
                HStack(spacing: 6) {
                    Text("Try it")
                        .textRole(.title)
                        .foregroundStyle(Palette.ink)
                        .accessibilityAddTraits(.isHeader)
                    HelpButton(text: "A request as Autopilot reads it: the facts first, then what the AI wrote. Edit anything; nothing is kept.")
                }
                .padding(.horizontal, 20)
                .padding(.top, 4)
                if ap.settings != nil && !ap.modelReady {
                    IntegrationBanner(text: "Download the model first.", kind: .warning)
                        .padding(.horizontal, 16)
                        .padding(.top, 12)
                        .accessibilityIdentifier("tryNoModel")
                }

                ScrollView(.horizontal, showsIndicators: false) {
                    HStack(spacing: 8) {
                        ForEach(AutopilotText.examples, id: \.title) { e in
                            ExampleChip(title: e.title, selected: example == e.title) {
                                example = e.title
                                text = e.situation
                                ap.clearEvaluation()
                            }
                            .accessibilityIdentifier("example:\(e.title)")
                        }
                    }
                    .padding(.horizontal, 16)
                    .padding(.vertical, 2)
                }
                .padding(.top, 12)

                SectionHeader("Comes in", glyph: .bell)
                TextEditor(text: Binding(get: { text }, set: { new in
                    text = String(new.prefix(4_000))
                    example = nil
                }))
                .font(RFont.mono(13))
                .foregroundStyle(Palette.ink)
                .scrollContentBackground(.hidden)
                .autocorrectionDisabled()
                .textInputAutocapitalization(.never)
                .focused($editing)
                .frame(minHeight: 170)
                .padding(10)
                .background(Palette.paperRaised, in: RoundedRectangle(cornerRadius: 20, style: .continuous))
                .overlay(RoundedRectangle(cornerRadius: 20, style: .continuous).strokeBorder(editing ? Palette.violet : Palette.rule, lineWidth: editing ? 1.5 : 1))
                .padding(.horizontal, 16)
                .accessibilityLabel("The request")
                .accessibilityIdentifier("situation")

                Button {
                    editing = false
                    Task { await ap.evaluate(profileId: profile?.id, situation: text) }
                } label: {
                    if ap.evaluating { ProgressView().tint(Palette.onInverse) } else { Text("Ask Autopilot") }
                }
                .buttonStyle(SignalButtonLook(kind: .inverse))
                .frame(height: 54)
                .disabled(ap.evaluating || text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
                .padding(.horizontal, 16)
                .padding(.top, 14)
                .accessibilityIdentifier("evaluate")
                if let error = ap.error {
                    IntegrationBanner(text: error, kind: .error).padding(.horizontal, 16).padding(.top, 10).accessibilityIdentifier("tryError")
                }

                if let evaluation = ap.evaluation {
                    VerdictSection(s: evaluation)
                        .id(evaluation.reason + "\(evaluation.pApprove)")
                        .transition(.opacity.combined(with: .offset(y: 24)))
                }
            }
            .animation(.spring(response: 0.45, dampingFraction: 0.85), value: ap.evaluation)
            .frame(maxWidth: 680)
            .frame(maxWidth: .infinity)
            .padding(.bottom, 40)
        }
        .scrollDismissesKeyboard(.interactively)
        .background(Palette.paper.ignoresSafeArea())
        .navigationTitle("")
        .navigationBarTitleDisplayMode(.inline)
        .toolbar {
            ToolbarItem(placement: .topBarTrailing) {
                Menu {
                    ForEach(ap.profiles, id: \.id) { p in
                        Button(untrustedName(p.name, limit: 40)) {
                            chosenProfile = p.id
                            ap.clearEvaluation()
                        }
                        .accessibilityIdentifier("tryProfile:\(p.id)")
                    }
                } label: {
                    HStack(spacing: 6) {
                        GlyphIcon(.person, size: 16)
                        Text(profile.map { untrustedName($0.name, limit: 24) } ?? "Profile").font(RFont.sans(15, .semibold)).lineLimit(1)
                        GlyphIcon(.chevronDown, size: 13)
                    }
                    .foregroundStyle(Palette.ink)
                    .padding(.horizontal, 6)
                }
                .accessibilityLabel("Profile")
                .accessibilityValue(profile.map { untrustedName($0.name) } ?? "")
                .accessibilityIdentifier("tryProfileMenu")
            }
            ToolbarItemGroup(placement: .keyboard) {
                Spacer()
                Button("Done") { editing = false }
            }
        }
        .task {
            ap.bind(model)
            ap.clearEvaluation()
            await ap.refresh()
        }
    }

}

/// An example to try: a chip, filled in ink when chosen.
private struct ExampleChip: View {
    var title: String
    var selected: Bool
    var action: () -> Void
    @Environment(\.feedback) private var feedback

    var body: some View {
        Button {
            feedback.play(.selection)
            action()
        } label: {
            Text(title)
                .font(RFont.sans(14.5, .semibold))
                .foregroundStyle(selected ? Palette.onInverse : Palette.ink)
                .padding(.horizontal, 14)
                .frame(minHeight: 38)
                .background(selected ? Palette.inverse : Palette.paperRaised, in: Capsule())
                .overlay { if !selected { Capsule().strokeBorder(Palette.rule, lineWidth: 1) } }
                .contentShape(Capsule())
        }
        .buttonStyle(PressDimsStyle())
        .accessibilityAddTraits(selected ? .isSelected : [])
    }
}

/// The answer (the Autopilot board, 06): what Autopilot does with it on one line, its verdict and how sure, then why.
private struct VerdictSection: View {
    var s: SuggestionView

    var body: some View {
        let tint = s.judged ? AutopilotStyle.tint(s.verdict) : Palette.ink2
        VStack(alignment: .leading, spacing: 0) {
            HStack(spacing: 8) {
                GlyphIcon(.arrowForward, size: 15).rotationEffect(.degrees(90))
                Text(Self.decides(s)).font(RFont.sans(13.5, .semibold))
                Rectangle().fill(Palette.rule).frame(height: 1).padding(.leading, 6)
            }
            .foregroundStyle(tint)
            .padding(.horizontal, 24)
            .padding(.top, 18)
            .padding(.bottom, 8)
            RaisedGroup(inset: 14) {
                HStack(spacing: 14) {
                    ProgressRing(fraction: Double(s.verdict == .deny ? s.pDeny : s.pApprove), tint: tint, size: 52, stroke: 5) {
                        Image(systemName: AutopilotStyle.symbol(s.verdict)).font(.system(size: 18, weight: .bold)).foregroundStyle(tint)
                    }
                    VStack(alignment: .leading, spacing: 2) {
                        (Text(s.judged ? AutopilotText.verdictWord(s.verdict) : "Leave it to you").font(RFont.display(24))
                            + Text(s.judged ? " \(AutopilotText.percent(s.verdict == .deny ? s.pDeny : s.pApprove))" : "").font(RFont.display(24, target: true)))
                            .foregroundStyle(tint)
                            .lineLimit(1)
                            .minimumScaleFactor(0.7)
                            .accessibilityIdentifier("verdictWord")
                        if s.judged {
                            Text("\(AutopilotText.percent(s.confidence)) confident").textRole(.meta).foregroundStyle(Palette.inkMeta)
                        }
                    }
                }
                .padding(14)
                .frame(maxWidth: .infinity, alignment: .leading)
                if s.judged {
                    VStack(spacing: 10) {
                        ProbabilityRow(label: "Approve", p: s.pApprove, tint: Palette.success)
                        ProbabilityRow(label: "Deny", p: s.pDeny, tint: Palette.danger)
                        ProbabilityRow(label: "Confidence", p: s.confidence, tint: Palette.violet)
                    }
                    .padding(14)
                }
            }
            if !s.reason.isEmpty || !s.neighbours.isEmpty || !AutopilotText.suggestionNotes(s).isEmpty {
                SectionHeader("Why")
                RaisedGroup(inset: 46) {
                    if !s.reason.isEmpty {
                        GrantFact(glyph: .circleCheck, text: untrusted(s.reason))
                            .accessibilityIdentifier("verdictReason")
                    }
                    ForEach(Array(s.neighbours.enumerated()), id: \.offset) { _, n in
                        AutopilotNeighbourRow(neighbour: n).padding(.horizontal, 14).padding(.vertical, 6)
                    }
                    ForEach(AutopilotText.suggestionNotes(s), id: \.self) { note in
                        AutopilotNoteRow(text: note).padding(.horizontal, 14).padding(.vertical, 10)
                    }
                }
            }
        }
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("verdict")
    }

    /// "Autopilot allows it", "Autopilot denies it", "Not sure: it waits for you".
    static func decides(_ s: SuggestionView) -> String {
        guard s.judged else { return "Not sure: it waits for you" }
        return s.verdict == .deny ? "Autopilot denies it" : "Autopilot allows it"
    }
}
