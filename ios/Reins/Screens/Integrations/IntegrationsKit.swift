import SwiftUI
import UIKit

// The pieces the integration screens share: Zeron's inset grouped lists, banners, buttons that show work in
// progress, status pills, secret fields with a paste button, and an account's row.

extension View {
    /// An inset grouped list on the app background, rows on the elevated surface (Zeron's settings lists).
    func integrationList() -> some View {
        listStyle(.insetGrouped)
            .scrollContentBackground(.hidden)
            .background(Palette.background.ignoresSafeArea())
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

/// The small heading above a group of rows.
struct GroupHeading: View {
    var title: String

    init(_ title: String) { self.title = title }

    var body: some View {
        SectionHeader(title)
            .textCase(nil)
            .padding(.horizontal, -4)
    }
}

/// The explanation under a group.
struct GroupFootnote: View {
    var text: String

    init(_ text: String) { self.text = text }

    var body: some View {
        Text(text)
            .font(RFont.sans(13))
            .foregroundStyle(Palette.secondary)
            .fixedSize(horizontal: false, vertical: true)
    }
}

/// A message in a tinted box: what is going on, a warning, or what went wrong.
struct IntegrationBanner: View {
    enum Kind { case notice, warning, error, success }
    var text: String
    var kind: Kind = .notice

    private var tint: Color {
        switch kind {
        case .notice: Palette.accent
        case .warning: Palette.warning
        case .error: Palette.danger
        case .success: Palette.success
        }
    }

    private var symbol: String {
        switch kind {
        case .notice: "info.circle.fill"
        case .warning: "exclamationmark.triangle.fill"
        case .error: "exclamationmark.octagon.fill"
        case .success: "checkmark.circle.fill"
        }
    }

    var body: some View {
        HStack(alignment: .firstTextBaseline, spacing: 10) {
            Image(systemName: symbol)
                .font(.system(size: 15, weight: .semibold))
                .foregroundStyle(tint)
                .accessibilityHidden(true)
            Text(text)
                .font(RFont.sans(14.5))
                .foregroundStyle(Palette.text)
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
            .font(RFont.sans(12.5, .medium))
            .foregroundStyle(tint)
            .padding(.horizontal, 9)
            .padding(.vertical, 3.5)
            .background(tint.opacity(0.13), in: Capsule())
            .lineLimit(1)
            .fixedSize()
    }
}

/// A full-width capsule button that dims while disabled, shows a spinner while its work runs, and plays the tap.
struct ActionButton: View {
    enum Kind { case primary, accent, secondary, ghost, destructive }

    var title: String
    var symbol: String? = nil
    var kind: Kind = .primary
    var busy = false
    var enabled = true
    var compact = false
    var action: () -> Void

    @Environment(\.feedback) private var feedback

    private var colors: (fg: Color, bg: Color) {
        switch kind {
        case .primary: (Palette.background, Palette.text)
        case .accent: (.white, Palette.accent)
        case .secondary: (Palette.text, Palette.controlFill)
        case .ghost: (Palette.secondary, .clear)
        case .destructive: (Palette.danger, Palette.danger.opacity(0.12))
        }
    }

    var body: some View {
        Button {
            feedback.play(.tap)
            action()
        } label: {
            HStack(spacing: 8) {
                if busy {
                    ProgressView().tint(colors.fg).controlSize(.small)
                } else if let symbol {
                    Image(systemName: symbol).font(.system(size: compact ? 13 : 15, weight: .semibold))
                }
                Text(title)
                    .font(RFont.sans(compact ? 14 : 16, .semibold))
                    .multilineTextAlignment(.center)
                    .lineLimit(2)
            }
            .foregroundStyle(colors.fg)
            .padding(.horizontal, compact ? 14 : 18)
            .padding(.vertical, compact ? 7 : 12)
            .frame(maxWidth: compact ? nil : .infinity, minHeight: compact ? 34 : 50)
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
        .background(Palette.elevated, in: RoundedRectangle(cornerRadius: 16, style: .continuous))
        .overlay(RoundedRectangle(cornerRadius: 16, style: .continuous).strokeBorder(Palette.hairline, lineWidth: 1))
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
        .tint(Palette.accent)
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
            Text(hint)
                .font(RFont.sans(13))
                .foregroundStyle(Palette.secondary)
                .fixedSize(horizontal: false, vertical: true)
                .padding(.horizontal, 4)
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

/// How an account is doing, under its name.
struct AccountStatusLine: View {
    var status: GmailStatus?
    var needsAgain: String

    var body: some View {
        switch status {
        case nil:
            HStack(spacing: 6) {
                ProgressView().controlSize(.mini)
                Text("Checking…").font(RFont.sans(13)).foregroundStyle(Palette.secondary)
            }
        case .ready:
            StatusPill(text: "Connected", tint: Palette.success)
        case .needsConsent:
            Text(needsAgain).font(RFont.sans(13, .medium)).foregroundStyle(Palette.warning)
        case let .unavailable(message):
            Text(untrusted(message)).font(RFont.sans(13)).foregroundStyle(Palette.danger).lineLimit(3)
        }
    }
}

/// One connected account: who, how it is doing, "Allow again" when that helps, and removing it.
struct AccountRowView<Avatar: View>: View {
    var title: String
    var status: GmailStatus?
    var needsAgain: String
    var canAllowAgain: Bool
    var busy: Bool
    @ViewBuilder var avatar: Avatar
    var onAllowAgain: () -> Void
    var onRemove: () -> Void

    @Environment(\.feedback) private var feedback

    var body: some View {
        HStack(spacing: 14) {
            avatar
            VStack(alignment: .leading, spacing: 4) {
                Text(title)
                    .font(RFont.sans(16, .medium))
                    .foregroundStyle(Palette.text)
                    .lineLimit(1)
                    .truncationMode(.middle)
                    .environment(\.layoutDirection, .leftToRight)
                AccountStatusLine(status: status, needsAgain: needsAgain)
                if status == .needsConsent && canAllowAgain {
                    ActionButton(title: "Allow again", kind: .secondary, enabled: !busy, compact: true, action: onAllowAgain)
                        .padding(.top, 4)
                        .accessibilityIdentifier("reconnect:\(title)")
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            Button {
                feedback.play(.tap)
                onRemove()
            } label: {
                Image(systemName: "trash")
                    .font(.system(size: 17, weight: .medium))
                    .foregroundStyle(Palette.danger)
                    .frame(width: 40, height: 40)
                    .contentShape(Circle())
            }
            .buttonStyle(.borderless)
            .disabled(busy)
            .accessibilityLabel("Remove \(title)")
            .accessibilityIdentifier("removeAccount:\(title)")
        }
        .padding(.vertical, 4)
        .swipeActions(edge: .trailing) {
            Button("Remove", systemImage: "trash", role: .destructive, action: onRemove)
        }
        .accessibilityElement(children: .contain)
    }
}

/// Nothing connected yet, and what connecting does.
struct NoAccountsRow: View {
    var title: String
    var message: String

    var body: some View {
        VStack(alignment: .leading, spacing: 3) {
            Text(title).font(RFont.sans(16, .medium)).foregroundStyle(Palette.text)
            Text(message)
                .font(RFont.sans(13))
                .foregroundStyle(Palette.secondary)
                .fixedSize(horizontal: false, vertical: true)
        }
        .padding(.vertical, 6)
        .accessibilityElement(children: .combine)
        .accessibilityIdentifier("noAccounts")
    }
}
