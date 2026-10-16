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
            VStack(alignment: .leading, spacing: 20) {
                VStack(alignment: .leading, spacing: 4) {
                    AskText([AskText.Run(text: "Try "), AskText.Run(text: "it", target: true)], role: .title)
                        .foregroundStyle(Palette.ink)
                        .accessibilityAddTraits(.isHeader)
                    Text("A request as Autopilot reads it: the facts first, then what the AI wrote. Edit anything; nothing is kept.")
                        .textRole(.body)
                        .foregroundStyle(Palette.ink2)
                        .fixedSize(horizontal: false, vertical: true)
                }
                .padding(.horizontal, 8)
                .padding(.top, 4)
                if ap.settings != nil && !ap.modelReady {
                    IntegrationBanner(text: "Download the model first.", kind: .warning)
                        .accessibilityIdentifier("tryNoModel")
                }

                chips("Examples") {
                    ForEach(AutopilotText.examples, id: \.title) { e in
                        AutopilotChip(title: e.title, selected: example == e.title, identifier: "example:\(e.title)") {
                            example = e.title
                            text = e.situation
                            ap.clearEvaluation()
                        }
                    }
                }

                if ap.profiles.count > 1 {
                    chips("Profile") {
                        ForEach(ap.profiles, id: \.id) { p in
                            AutopilotChip(title: untrustedName(p.name, limit: 40), selected: p.id == profile?.id, identifier: "tryProfile:\(p.id)") {
                                chosenProfile = p.id
                                ap.clearEvaluation()
                            }
                        }
                    }
                }

                VStack(alignment: .leading, spacing: 8) {
                    SectionHeader("The request").padding(.horizontal, 12)
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
                    .frame(minHeight: 200)
                    .padding(10)
                    .background(Palette.paperRaised, in: RoundedRectangle(cornerRadius: 16, style: .continuous))
                    .overlay(RoundedRectangle(cornerRadius: 16, style: .continuous).strokeBorder(editing ? Palette.violet : Palette.rule, lineWidth: editing ? 1.5 : 1))
                    .accessibilityLabel("The request")
                    .accessibilityIdentifier("situation")
                }

                VStack(spacing: 12) {
                    Button {
                        editing = false
                        Task { await ap.evaluate(profileId: profile?.id, situation: text) }
                    } label: {
                        if ap.evaluating { ProgressView().tint(Palette.onInverse) } else { Text("Ask Autopilot") }
                    }
                    .buttonStyle(SignalButtonLook(kind: .inverse))
                    .frame(height: 54)
                    .disabled(ap.evaluating || text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
                    .accessibilityIdentifier("evaluate")
                    if let error = ap.error {
                        IntegrationBanner(text: error, kind: .error).accessibilityIdentifier("tryError")
                    }
                }

                if let evaluation = ap.evaluation {
                    VerdictCard(s: evaluation)
                        .id(evaluation.reason + "\(evaluation.pApprove)")
                        .transition(.opacity.combined(with: .offset(y: 24)))
                }
            }
            .animation(.spring(response: 0.45, dampingFraction: 0.85), value: ap.evaluation)
            .frame(maxWidth: 680)
            .frame(maxWidth: .infinity)
            .padding(.horizontal, 16)
            .padding(.bottom, 40)
        }
        .scrollDismissesKeyboard(.interactively)
        .background(Palette.paper.ignoresSafeArea())
        .navigationTitle("")
        .navigationSubtitle(profile.map { untrustedName($0.name) } ?? "")
        .navigationBarTitleDisplayMode(.inline)
        .toolbar {
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

    private func chips<Content: View>(_ title: String, @ViewBuilder _ content: () -> Content) -> some View {
        VStack(alignment: .leading, spacing: 8) {
            SectionHeader(title).padding(.horizontal, 12)
            ScrollView(.horizontal, showsIndicators: false) {
                HStack(spacing: 8) { content() }.padding(.horizontal, 4).padding(.vertical, 2)
            }
            .scrollClipDisabled()
        }
    }
}

/// The answer: a big verdict, the probabilities as bars, the reason, the neighbours, and what held it back.
private struct VerdictCard: View {
    var s: SuggestionView

    var body: some View {
        let tint = s.judged ? AutopilotStyle.tint(s.verdict) : Palette.inkMeta
        VStack(alignment: .leading, spacing: 0) {
            HStack(spacing: 16) {
                ProgressRing(fraction: Double(s.verdict == .deny ? s.pDeny : s.pApprove), tint: tint, size: 64, stroke: 6) {
                    Image(systemName: AutopilotStyle.symbol(s.verdict)).font(.system(size: 22, weight: .bold)).foregroundStyle(tint)
                }
                VStack(alignment: .leading, spacing: 2) {
                    Text("Autopilot would").textRole(.label).foregroundStyle(Palette.inkMeta)
                    // "Deny *91%*": the verdict in the serif, how sure in italic.
                    (Text(s.judged ? AutopilotText.verdictWord(s.verdict) : "Leave it to you").font(RFont.serif(38))
                        + Text(s.judged ? " \(AutopilotText.percent(s.verdict == .deny ? s.pDeny : s.pApprove))" : "").font(RFont.serif(38, italic: true)))
                        .foregroundStyle(tint)
                        .lineLimit(1)
                        .minimumScaleFactor(0.7)
                        .accessibilityIdentifier("verdictWord")
                    if s.judged {
                        Text("\(AutopilotText.percent(s.confidence)) confident").textRole(.meta).foregroundStyle(Palette.inkMeta)
                    }
                }
            }
            if s.judged {
                VStack(spacing: 10) {
                    ProbabilityRow(label: "Approve", p: s.pApprove, tint: Palette.success)
                    ProbabilityRow(label: "Deny", p: s.pDeny, tint: Palette.danger)
                    ProbabilityRow(label: "Confidence", p: s.confidence, tint: Palette.violet)
                }
                .padding(.top, 18)
            }
            if !s.reason.isEmpty {
                Text(untrusted(s.reason))
                    .font(RFont.sans(15))
                    .foregroundStyle(Palette.ink)
                    .fixedSize(horizontal: false, vertical: true)
                    .padding(.top, 16)
                    .accessibilityIdentifier("verdictReason")
            }
            if !s.neighbours.isEmpty {
                AutopilotCaption("Like these decisions of yours").padding(.top, 16).padding(.bottom, 2)
                ForEach(Array(s.neighbours.enumerated()), id: \.offset) { _, n in AutopilotNeighbourRow(neighbour: n) }
            }
            ForEach(AutopilotText.suggestionNotes(s), id: \.self) { note in
                AutopilotNoteRow(text: note).padding(.top, 12)
            }
        }
        .padding(18)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(Palette.paperRaised, in: RoundedRectangle(cornerRadius: 22, style: .continuous))
        .overlay(RoundedRectangle(cornerRadius: 22, style: .continuous).strokeBorder(Palette.rule, lineWidth: 1))
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("verdict")
    }
}
