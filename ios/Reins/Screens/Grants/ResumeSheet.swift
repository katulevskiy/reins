import SwiftUI

/// Asks how long to resume an ended grant for, or to extend a running one by. The quick choices are up front
/// (Once | 1 hour | 1 day | Custom); "More options" (like on an approval) holds a custom time, the number of uses, and
/// who or what the grant covers. Extending only asks for the time: the grant keeps its uses and scope.
struct ResumeSheet: View {
    var grant: GrantView
    /// A running grant: Extend, from now.
    var extending = false
    var onResume: (_ seconds: Int64, _ standing: StandingGrant?) -> Void
    @Environment(\.dismiss) private var dismiss
    @Environment(\.feedback) private var feedback
    @State private var draft: ResumeDraft
    @State private var error: String?
    @State private var detent: PresentationDetent = .medium

    init(grant: GrantView, extending: Bool = false, onResume: @escaping (_ seconds: Int64, _ standing: StandingGrant?) -> Void) {
        self.grant = grant
        self.extending = extending
        self.onResume = onResume
        _draft = State(initialValue: initialResumeDraft(grant))
    }

    private var editable: Bool { grant.editableScope != nil }
    private var send: Bool { grant.action == "send" }

    var body: some View {
        NavigationStack {
            ScrollView {
                VStack(alignment: .leading, spacing: 0) {
                    // "Extend *for*": the sheet's title in the serif, one italic word; what it is for under it.
                    AskText([AskText.Run(text: extending ? "Extend " : "Resume "), AskText.Run(text: "for", target: true)], role: .sheetTitle)
                        .foregroundStyle(Palette.ink)
                        .accessibilityAddTraits(.isHeader)
                    Text(verbatim: untrusted(grant.summary))
                        .textRole(.body)
                        .foregroundStyle(Palette.ink2)
                        .fixedSize(horizontal: false, vertical: true)
                        .padding(.top, 6)
                    durations
                        .padding(.top, 20)

                    if extending {
                        if draft.moreOpen { customTime.transition(.opacity.combined(with: .move(edge: .top))) }
                    } else {
                        moreToggle
                        if draft.moreOpen { more.transition(.opacity.combined(with: .move(edge: .top))) }
                    }

                    if let error {
                        FormBanner(text: error).padding(.top, 14).accessibilityIdentifier("resumeInvalid")
                    }
                }
                .padding(.horizontal, 24)
                .padding(.top, 28)
                .padding(.bottom, 20)
                .animation(.smooth(duration: 0.25), value: draft.moreOpen)
            }
            .scrollDismissesKeyboard(.interactively)
            .safeAreaInset(edge: .bottom) {
                HStack(spacing: 10) {
                    Button("Cancel") { dismiss() }
                        .buttonStyle(SignalButtonLook(kind: .outline))
                    Button(action: confirm) {
                        Label(extending ? "Extend" : "Resume", systemImage: BiometricSymbol.name)
                    }
                    .buttonStyle(SignalButtonLook(kind: .inverse))
                    .accessibilityIdentifier("confirmResume")
                }
                .frame(height: 54)
                .padding(.horizontal, 22)
                .padding(.top, 10)
                .padding(.bottom, 12)
                .background(Palette.paper.opacity(0.001))
            }
            .toolbar(.hidden, for: .navigationBar)
            .background(Palette.paper.ignoresSafeArea())
        }
        .presentationDetents([.medium, .large], selection: $detent)
        .presentationDragIndicator(.visible)
        .presentationBackground(Palette.paper)
        .accessibilityIdentifier(extending ? "extendDialog" : "resumeDialog")
    }

    private var custom: Bool { !draft.customAmount.trimmingCharacters(in: .whitespaces).isEmpty }

    /// Once | 1 hour | 1 day | Custom ▾ (7 days, 30 days, another time). Once only where the grant can be limited to a
    /// use; never when extending (the grant keeps its uses).
    private var durations: some View {
        let allMail = editable && draft.allMail
        let periods = resumePeriods(allMail: grant.allMail || allMail)
        typealias Option = DurationControl<ResumePick>.Option
        var inline: [Option] = []
        if !extending {
            inline.append(Option(value: .once, label: "Once", id: "once", enabled: editable))
        }
        for p in [ResumePeriod.hour, .day] {
            inline.append(Option(value: .period(p), label: p == .day ? "1 day" : p.label, id: "\(p)"))
        }
        let more: [Option] = [ResumePeriod.week, .month].map { p in
            Option(value: .period(p), label: p.label, id: "\(p)", enabled: periods.contains(p))
        }
        return DurationControl(
            inline: inline,
            more: more,
            selection: draft.once ? .once : .period(draft.period),
            customChosen: custom,
            idPrefix: "period",
            onOther: {
                edit { $0.moreOpen = true }
                detent = .large
            },
            onSelect: { pick in
                edit {
                    $0.customAmount = ""
                    switch pick {
                    case .once:
                        $0.once = true
                        $0.period = .day
                    case let .period(p):
                        $0.once = false
                        $0.period = p
                    }
                }
            }
        )
    }

    /// Extending: only a typed time.
    private var customTime: some View {
        VStack(alignment: .leading, spacing: 0) {
            label("Custom time")
            HStack(spacing: 10) {
                TextField("e.g. 90", text: digits(\.customAmount))
                    .keyboardType(.numberPad)
                    .fieldWell()
                    .frame(width: 110)
                    .accessibilityLabel("Custom time")
                    .accessibilityIdentifier("customAmount")
                ChipFlow {
                    ForEach(CustomUnit.allCases, id: \.self) { unit in
                        OptionChip(unit.label, selected: draft.customUnit == unit) { edit { $0.customUnit = unit } }
                            .accessibilityIdentifier("unit:\(unit)")
                    }
                }
            }
        }
    }

    private var moreToggle: some View {
        Button {
            feedback.play(.expand(!draft.moreOpen))
            edit { $0.moreOpen.toggle() }
            if draft.moreOpen { detent = .large }
        } label: {
            HStack {
                Text("More options").font(TextRole.row.font())
                Spacer()
                GlyphIcon(.chevronRight, size: 14)
                    .foregroundStyle(Palette.inkMeta)
                    .rotationEffect(.degrees(draft.moreOpen ? 90 : 0))
            }
            .foregroundStyle(Palette.ink)
            .padding(.vertical, 14)
            .overlay(alignment: .bottom) { Rectangle().fill(Palette.rule).frame(height: 1) }
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .padding(.top, 6)
        .accessibilityIdentifier("resumeMore")
        .accessibilityValue(draft.moreOpen ? "Open" : "Closed")
    }

    @ViewBuilder private var more: some View {
        VStack(alignment: .leading, spacing: 0) {
            label("Custom time")
            HStack(spacing: 10) {
                TextField("e.g. 90", text: digits(\.customAmount))
                    .keyboardType(.numberPad)
                    .fieldWell()
                    .frame(width: 110)
                    .accessibilityLabel("Custom time")
                    .accessibilityIdentifier("customAmount")
                ChipFlow {
                    ForEach(CustomUnit.allCases, id: \.self) { unit in
                        OptionChip(unit.label, selected: draft.customUnit == unit) { edit { $0.customUnit = unit } }
                            .accessibilityIdentifier("unit:\(unit)")
                    }
                }
            }

            label("Uses")
            HStack(spacing: 8) {
                OptionChip("No limit", selected: !draft.limitUses) { edit { $0.limitUses = false } }
                    .accessibilityIdentifier("usesNone")
                OptionChip("Limit to", selected: draft.limitUses) { edit { $0.limitUses = true } }
                    .accessibilityIdentifier("usesLimit")
                if draft.limitUses {
                    TextField("5", text: digits(\.uses))
                        .keyboardType(.numberPad)
                        .fieldWell()
                        .frame(width: 90)
                        .accessibilityLabel("Number of uses")
                        .accessibilityIdentifier("resumeUses")
                }
            }

            if editable {
                if !send {
                    label("Which emails?")
                    HStack(spacing: 8) {
                        OptionChip("Specific senders", selected: !draft.allMail, symbol: "at") { edit { $0.allMail = false } }
                            .accessibilityIdentifier("resumeScope:specific")
                        OptionChip("All mail", selected: draft.allMail, symbol: "tray") {
                            edit {
                                $0.allMail = true
                                if $0.period == .month { $0.period = .week }
                            }
                        }
                        .accessibilityIdentifier("resumeScope:all")
                    }
                }
                if !draft.allMail {
                    label(send ? "Send to" : "From")
                    TextField(send ? "a@b.com, @b.com" : "alerts@bank.com, @bank.com", text: limited(\.partiesText, 1000), axis: .vertical)
                        .lineLimit(1...4)
                        .textInputAutocapitalization(.never)
                        .autocorrectionDisabled()
                        .keyboardType(.emailAddress)
                        .fieldWell(mono: true)
                        .accessibilityLabel(send ? "Send to" : "From")
                        .accessibilityIdentifier("resumeParties")
                    label("Subject contains (optional)")
                    TextField("e.g. statement", text: limited(\.subject, 200))
                        .fieldWell()
                        .accessibilityLabel("Subject contains")
                        .accessibilityIdentifier("resumeSubject")
                }
            } else {
                Text("Comes back as it was.")
                    .font(RFont.sans(13.5))
                    .foregroundStyle(Palette.inkMeta)
                    .padding(.top, 14)
            }
        }
        .padding(.bottom, 4)
    }

    private func label(_ text: String) -> some View {
        Text(text)
            .textRole(.label)
            .foregroundStyle(Palette.inkMeta)
            .padding(.top, 18)
            .padding(.bottom, 8)
    }

    private func edit(_ change: (inout ResumeDraft) -> Void) {
        change(&draft)
        error = nil
    }

    private func digits(_ key: WritableKeyPath<ResumeDraft, String>) -> Binding<String> {
        Binding(get: { draft[keyPath: key] }, set: { text in edit { $0[keyPath: key] = String(text.filter { $0.isASCII && $0.isNumber }.prefix(4)) } })
    }

    private func limited(_ key: WritableKeyPath<ResumeDraft, String>, _ max: Int) -> Binding<String> {
        Binding(get: { draft[keyPath: key] }, set: { text in edit { $0[keyPath: key] = String(text.prefix(max)) } })
    }

    private func confirm() {
        switch extending ? buildExtend(grant, draft) : buildResume(grant, draft) {
        case let .invalid(message):
            error = message
        case let .ok(seconds, standing):
            dismiss()
            onResume(seconds, standing)
        }
    }
}
