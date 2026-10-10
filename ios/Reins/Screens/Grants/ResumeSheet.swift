import SwiftUI

/// Asks how long to resume an ended grant for. The quick choices are up front; "More options" (like on an approval)
/// holds a custom time, the number of uses, and who or what the grant covers.
struct ResumeSheet: View {
    var grant: GrantView
    var onResume: (_ seconds: Int64, _ standing: StandingGrant?) -> Void
    @Environment(\.dismiss) private var dismiss
    @Environment(\.feedback) private var feedback
    @State private var draft: ResumeDraft
    @State private var error: String?
    @State private var detent: PresentationDetent = .medium

    init(grant: GrantView, onResume: @escaping (_ seconds: Int64, _ standing: StandingGrant?) -> Void) {
        self.grant = grant
        self.onResume = onResume
        _draft = State(initialValue: initialResumeDraft(grant))
    }

    private var editable: Bool { grant.editableScope != nil }
    private var send: Bool { grant.action == "send" }

    var body: some View {
        NavigationStack {
            ScrollView {
                VStack(alignment: .leading, spacing: 0) {
                    Text(untrusted(grant.summary))
                        .font(RFont.sans(16, .medium))
                        .foregroundStyle(Palette.text)
                    Text("\(untrusted(grant.connectionLabel)) may use it again for:")
                        .font(RFont.sans(14.5))
                        .foregroundStyle(Palette.secondary)
                        .padding(.top, 8)
                    ChipFlow {
                        ForEach(resumePeriods(allMail: editable && draft.allMail), id: \.self) { p in
                            OptionChip(p.label, selected: !custom && draft.period == p) {
                                edit { $0.period = p; $0.customAmount = "" }
                            }
                            .accessibilityIdentifier("period:\(p)")
                        }
                    }
                    .padding(.top, 14)

                    moreToggle
                    if draft.moreOpen { more.transition(.opacity.combined(with: .move(edge: .top))) }

                    if let error {
                        FormBanner(text: error).padding(.top, 14).accessibilityIdentifier("resumeInvalid")
                    }
                }
                .padding(.horizontal, 22)
                .padding(.top, 4)
                .padding(.bottom, 20)
                .animation(.smooth(duration: 0.25), value: draft.moreOpen)
            }
            .scrollDismissesKeyboard(.interactively)
            .safeAreaInset(edge: .bottom) {
                HStack(spacing: 12) {
                    Button("Cancel") { dismiss() }
                        .buttonStyle(CapsuleButtonStyle(kind: .secondary, height: 50))
                    Button("Resume", action: confirm)
                        .buttonStyle(CapsuleButtonStyle(kind: .primary, height: 50))
                        .accessibilityIdentifier("confirmResume")
                }
                .padding(.horizontal, 22)
                .padding(.top, 10)
                .padding(.bottom, 12)
                .background(Palette.background.opacity(0.001))
            }
            .navigationTitle("Resume this grant?")
            .navigationBarTitleDisplayMode(.inline)
            .background(Palette.background.ignoresSafeArea())
        }
        .presentationDetents([.medium, .large], selection: $detent)
        .presentationDragIndicator(.visible)
        .presentationBackground(Palette.background)
        .accessibilityIdentifier("resumeDialog")
    }

    private var custom: Bool { !draft.customAmount.trimmingCharacters(in: .whitespaces).isEmpty }

    private var moreToggle: some View {
        Button {
            feedback.play(.expand(!draft.moreOpen))
            edit { $0.moreOpen.toggle() }
            if draft.moreOpen { detent = .large }
        } label: {
            HStack {
                Text("More options").font(RFont.sans(15.5, .medium))
                Spacer()
                Image(systemName: "chevron.right")
                    .font(.system(size: 14, weight: .semibold))
                    .rotationEffect(.degrees(draft.moreOpen ? 90 : 0))
            }
            .foregroundStyle(Palette.accent)
            .padding(.vertical, 12)
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
                        OptionChip("Specific senders", selected: !draft.allMail) { edit { $0.allMail = false } }
                            .accessibilityIdentifier("resumeScope:specific")
                        OptionChip("All mail", selected: draft.allMail) {
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
                    .foregroundStyle(Palette.tertiary)
                    .padding(.top, 14)
            }
        }
        .padding(.bottom, 4)
    }

    private func label(_ text: String) -> some View {
        Text(text)
            .font(RFont.sans(13, .semibold))
            .foregroundStyle(Palette.secondary)
            .padding(.top, 16)
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
        switch buildResume(grant, draft) {
        case let .invalid(message):
            error = message
        case let .ok(seconds, standing):
            dismiss()
            onResume(seconds, standing)
        }
    }
}
