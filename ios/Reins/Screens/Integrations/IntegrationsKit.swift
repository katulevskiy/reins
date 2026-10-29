import SwiftUI
import UIKit

/// "Account connected", "Server connected": the explicit finish, quiet. A green check, the title, who it is (cleaned,
/// mono), Done and, where there can be more than one, Add another account.
struct IntegrationCompletion: View {
    var title: String
    var identity: String
    var onDone: () -> Void
    var onAnother: (() -> Void)?

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            GlyphIcon(.check, size: 22)
                .foregroundStyle(.white)
                .frame(width: 44, height: 44)
                .background(Palette.success, in: Circle())
                .accessibilityHidden(true)
            Text(title).textRole(.quietTitle).foregroundStyle(Palette.ink).accessibilityAddTraits(.isHeader)
            Text(verbatim: untrusted(identity))
                .font(RFont.mono(14))
                .foregroundStyle(Palette.ink2)
                .fixedSize(horizontal: false, vertical: true)
                .textSelection(.enabled)
                .environment(\.layoutDirection, .leftToRight)
                .accessibilityIdentifier("connectedIdentity")
            Button("Done", action: onDone)
                .buttonStyle(SignalButtonStyle(.inverse))
                .padding(.top, 4)
                .accessibilityIdentifier("connectionDone")
            if let onAnother {
                Button("Add another account", action: onAnother)
                    .buttonStyle(SignalButtonStyle(.soft))
                    .accessibilityIdentifier("connectionAddAnother")
            }
        }
        .accessibilityContainer("integrationConnected")
    }
}

// The pieces the integration screens share: Zeron's inset grouped lists, banners, buttons that show work in
// progress, status pills, secret fields with a paste button, and an account's row.

extension View {
    /// A list of rows on paper with rule hairlines between them, never cards (the System board's lists).
    func integrationList() -> some View {
        listStyle(.plain)
            .scrollContentBackground(.hidden)
            .listRowSeparatorTint(Palette.rule)
            .background(Palette.paper.ignoresSafeArea())
            .listSectionSpacing(18)
            .environment(\.defaultMinListRowHeight, 44)
    }

    /// A row that is not a list cell: no background, no separators, the list's inset for its content.
    func plainListRow(top: CGFloat = 4, bottom: CGFloat = 4) -> some View {
        listRowBackground(Color.clear)
            .listRowSeparator(.hidden)
            .listRowInsets(EdgeInsets(top: top, leading: 20, bottom: bottom, trailing: 20))
    }
}

/// A message in a tinted box: what is going on, a warning, or what went wrong.
struct IntegrationBanner: View {
    enum Kind { case notice, warning, error, success }
    var text: String
    var kind: Kind = .notice

    private var tint: Color {
        switch kind {
        case .notice: Palette.violet
        case .warning: Palette.warning
        case .error: Palette.danger
        case .success: Palette.success
        }
    }

    private var glyph: Glyph {
        switch kind {
        case .notice: .help
        case .warning: .warning
        case .error: .circleAlert
        case .success: .circleCheck
        }
    }

    var body: some View {
        HStack(alignment: .firstTextBaseline, spacing: 10) {
            GlyphIcon(glyph, size: 16)
                .foregroundStyle(tint)
                .alignmentGuide(.firstTextBaseline) { $0[VerticalAlignment.center] + 5 }
            Text(text)
                .font(RFont.sans(14.5))
                .foregroundStyle(Palette.ink)
                .fixedSize(horizontal: false, vertical: true)
                .frame(maxWidth: .infinity, alignment: .leading)
        }
        .padding(.horizontal, 14)
        .padding(.vertical, 12)
        .background(tint.opacity(0.12), in: RoundedRectangle(cornerRadius: 16, style: .continuous))
        .accessibilityElement(children: .combine)
    }
}

/// A short status ("Connected", "Needs sign-in") on its colour.
struct StatusPill: View {
    var text: String
    var tint: Color

    var body: some View {
        Text(text)
            .font(TextRole.tag.font())
            .foregroundStyle(tint)
            .padding(.horizontal, 8)
            .padding(.vertical, 3.5)
            .background(tint.opacity(0.13), in: Capsule())
            .lineLimit(1)
            .fixedSize()
    }
}

/// A full-width capsule button that dims while disabled, shows a spinner while its work runs, and plays the default
/// tap unless its action plays something of its own.
struct ActionButton: View {
    enum Kind { case primary, accent, secondary, ghost, destructive }

    var title: String
    var symbol: String? = nil
    var kind: Kind = .primary
    var busy = false
    var enabled = true
    var compact = false
    var lineLimit: Int? = 2
    var action: () -> Void

    @Environment(\.feedback) private var feedback

    private var colors: (fg: Color, bg: Color) {
        switch kind {
        // The kit's buttons: ink for the one thing to do, violet only where a permission is given, the soft wash, red.
        case .primary: (Palette.onInverse, Palette.inverse)
        case .accent: (Palette.onInverse, Palette.inverse)
        case .secondary: (Palette.ink, Palette.paperSunken)
        case .ghost: (Palette.ink2, .clear)
        case .destructive: (Palette.danger, Palette.dangerWash)
        }
    }

    var body: some View {
        Button {
            action()
            feedback.defaultTap()
        } label: {
            HStack(spacing: 8) {
                if busy {
                    ProgressView().tint(colors.fg).controlSize(.small)
                } else if let symbol {
                    Image(systemName: symbol).font(.system(size: compact ? 13 : 15, weight: .semibold))
                }
                Text(title)
                    .font(compact ? TextRole.buttonSmall.font() : TextRole.button.font())
                    .multilineTextAlignment(.center)
                    .lineLimit(lineLimit)
            }
            .foregroundStyle(colors.fg)
            .padding(.horizontal, compact ? 14 : 18)
            .padding(.vertical, compact ? 7 : 12)
            .frame(maxWidth: compact ? nil : .infinity, minHeight: compact ? 36 : 54)
            .background(colors.bg, in: Capsule())
            .contentShape(Capsule())
        }
        .buttonStyle(PressDim())
        .disabled(!enabled || busy)
        .opacity(enabled || busy ? 1 : 0.4)
    }
}

/// Dims and shrinks a little while pressed; only the button's own shape takes the touch (inside list rows too).
struct PressDim: ButtonStyle {
    func makeBody(configuration: Configuration) -> some View {
        configuration.label
            .opacity(configuration.isPressed ? 0.7 : 1)
            .scaleEffect(configuration.isPressed ? 0.98 : 1)
            .animation(.spring(duration: 0.2), value: configuration.isPressed)
    }
}

/// A field on the elevated surface with a hairline edge (Android's `RTextField`).
struct InputWell<Field: View, Trailing: View>: View {
    @ViewBuilder var field: Field
    @ViewBuilder var trailing: Trailing

    var body: some View {
        HStack(spacing: 8) {
            field
                .frame(maxWidth: .infinity, minHeight: 48, alignment: .leading)
            trailing
        }
        .padding(.leading, 16)
        .padding(.trailing, 8)
        .background(Palette.paperRaised, in: RoundedRectangle(cornerRadius: 14, style: .continuous))
        .overlay(RoundedRectangle(cornerRadius: 14, style: .continuous).strokeBorder(Palette.rule, lineWidth: 1))
    }
}

extension InputWell where Trailing == EmptyView {
    init(@ViewBuilder field: () -> Field) {
        self.field = field()
        self.trailing = EmptyView()
    }
}

/// Puts the clipboard's text in a field without the "Allow Paste" prompt (the system's paste button).
struct PasteInto: View {
    var label = "Paste"
    var onPaste: (String) -> Void

    var body: some View {
        PasteButton(payloadType: String.self) { strings in
            guard let text = strings.first else { return }
            Task { @MainActor in onPaste(text.trimmingCharacters(in: .whitespacesAndNewlines)) }
        }
        .labelStyle(.iconOnly)
        .buttonBorderShape(.circle)
        .tint(Palette.ink2)
        .controlSize(.small)
        .accessibilityLabel(label)
    }
}

/// Removes what was pasted from the clipboard so a token does not lie around there.
@MainActor
func clearClipboard() {
    UIPasteboard.general.items = []
}

/// One secret typed or pasted in and sent once; it is cleared as soon as it is sent.
struct SecretForm: View {
    var placeholder: String
    var button: String
    var busy: Bool
    var hint: String
    var contentType: UITextContentType? = nil
    var onSubmit: (String) -> Void

    @State private var secret = ""
    @FocusState private var focused: Bool

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            InputWell {
                SecureField(placeholder, text: $secret)
                    .font(RFont.mono(15))
                    .textContentType(contentType)
                    .textInputAutocapitalization(.never)
                    .autocorrectionDisabled()
                    .focused($focused)
                    .submitLabel(.go)
                    .onSubmit(submit)
                    .disabled(busy)
                    .accessibilityIdentifier("secret")
            } trailing: {
                PasteInto { pasted in
                    secret = pasted
                    clearClipboard()
                }
                .disabled(busy)
            }
            if !hint.isEmpty { GroupFooter(hint) }
            ActionButton(title: button, busy: busy, enabled: !secret.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty, action: submit)
                .accessibilityIdentifier("connectSecret")
        }
    }

    private func submit() {
        guard !busy, !secret.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else { return }
        let sent = secret
        secret = ""
        focused = false
        onSubmit(sent)
    }
}

// MARK: Accounts

/// How an account is doing, beside its name: the green dot when it works; a problem in words, never just a colour.
struct AccountStatusLine: View {
    var status: GmailStatus?
    var needsAgain: String

    var body: some View {
        switch status {
        case nil:
            ProgressView().controlSize(.mini).accessibilityLabel("Checking")
        case .ready:
            OkDot()
        case .needsConsent:
            Text(needsAgain).textRole(.sub).foregroundStyle(Palette.warning).lineLimit(2)
        case let .unavailable(message):
            Text(untrusted(message)).textRole(.sub).foregroundStyle(Palette.danger).lineLimit(3)
        }
    }
}

/// One connected account (the kit's `acct`): its avatar, who (mono for a phone number), how it is doing, "Allow
/// again" when that helps, and removing it.
struct AccountRowView<Avatar: View>: View {
    var title: String
    var mono = false
    var status: GmailStatus?
    var needsAgain: String
    var canAllowAgain: Bool
    var busy: Bool
    @ViewBuilder var avatar: Avatar
    var onAllowAgain: () -> Void
    var onRemove: () -> Void

    @Environment(\.feedback) private var feedback

    private var problem: Bool {
        switch status {
        case .needsConsent?, .unavailable?: true
        default: false
        }
    }

    var body: some View {
        HStack(spacing: 12) {
            MarkPlate(size: 32) { avatar }
            VStack(alignment: .leading, spacing: 2) {
                Text(verbatim: title)
                    .font(mono ? RFont.mono(15) : TextRole.row.font())
                    .foregroundStyle(Palette.ink)
                    .lineLimit(1)
                    .truncationMode(mono ? .middle : .tail)
                    .environment(\.layoutDirection, .leftToRight)
                if problem { AccountStatusLine(status: status, needsAgain: needsAgain) }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            if !problem { AccountStatusLine(status: status, needsAgain: needsAgain) }
            // The one-tap fix beside the problem.
            if status == .needsConsent && canAllowAgain {
                Button("Allow again", action: onAllowAgain)
                    .buttonStyle(SignalButtonStyle(.soft, size: .small, fill: false))
                    .disabled(busy)
                    .accessibilityIdentifier("reconnect:\(title)")
            }
            QuietIconButton(.trash, label: "Remove \(title)", action: onRemove)
                .disabled(busy)
                .accessibilityIdentifier("removeAccount:\(title)")
        }
        .padding(.vertical, 9)
        .frame(minHeight: 56)
        .accessibilityElement(children: .contain)
    }
}

/// Nothing connected yet (what the service is for is behind the page's (?)).
struct NoAccountsRow: View {
    var title: String
    var message: String

    var body: some View {
        Text(title)
            .font(RFont.sans(15))
            .foregroundStyle(Palette.ink2)
            .frame(maxWidth: .infinity, minHeight: 56, alignment: .leading)
            .accessibilityIdentifier("noAccounts")
    }
}
