import SwiftUI

/// A heading, a card of rows and the explanation under it: Autopilot's pages are cards on the background (as in
/// the Android app and Zeron's settings), not a system list, so the hero and the mode picker sit among them.
struct AutopilotGroup<Content: View>: View {
    var header: String?
    var footer: String?
    @ViewBuilder var content: Content

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            if let header { SectionHeader(header).padding(.horizontal, 12) }
            VStack(alignment: .leading, spacing: 0) { content }
                .frame(maxWidth: .infinity, alignment: .leading)
                .background(Palette.elevated, in: RoundedRectangle(cornerRadius: 22, style: .continuous))
                .overlay(RoundedRectangle(cornerRadius: 22, style: .continuous).strokeBorder(Palette.hairline, lineWidth: 0.5))
                .clipShape(RoundedRectangle(cornerRadius: 22, style: .continuous))
            if let footer {
                Text(footer)
                    .font(RFont.sans(12.5))
                    .foregroundStyle(Palette.tertiary)
                    .fixedSize(horizontal: false, vertical: true)
                    .padding(.horizontal, 16)
            }
        }
    }
}

/// A hairline between rows of a card, inset to where the text starts.
struct RowDivider: View {
    var inset: CGFloat = 16

    var body: some View {
        Rectangle().fill(Palette.hairline).frame(height: 0.5).padding(.leading, inset)
    }
}

/// A tappable row of a card: a symbol on a tile, a title and subtitle, maybe a chevron.
struct AutopilotRow: View {
    var title: String
    var subtitle: String? = nil
    var symbol: String
    var tint: Color = Palette.accent
    var destructive = false
    var chevron = false
    var identifier: String
    var action: () -> Void
    @Environment(\.feedback) private var feedback

    var body: some View {
        Button {
            feedback.play(.tap)
            action()
        } label: {
            HStack(spacing: 14) {
                IconTile(symbol: symbol, tint: destructive ? Palette.danger : tint, size: 34)
                VStack(alignment: .leading, spacing: 2) {
                    Text(title).font(RFont.sans(16, .medium)).foregroundStyle(destructive ? Palette.danger : Palette.text)
                    if let subtitle {
                        Text(subtitle).font(RFont.sans(13)).foregroundStyle(Palette.secondary).fixedSize(horizontal: false, vertical: true)
                    }
                }
                Spacer(minLength: 6)
                if chevron {
                    Image(systemName: "chevron.right")
                        .font(.system(size: 13, weight: .semibold))
                        .foregroundStyle(Palette.tertiary)
                }
            }
            .padding(.horizontal, 16)
            .padding(.vertical, 12)
            .contentShape(Rectangle())
        }
        .buttonStyle(RowPress())
        .accessibilityElement(children: .combine)
        .accessibilityAddTraits(.isButton)
        .accessibilityIdentifier(identifier)
    }
}

/// A row's pressed state: the control fill behind it.
struct RowPress: ButtonStyle {
    func makeBody(configuration: Configuration) -> some View {
        configuration.label.background(configuration.isPressed ? Palette.controlFill : .clear)
    }
}

/// Turning a bypass on: how long (15, 30 or 60 minutes) and what it means. `who` names the AI for a connection's own
/// bypass; nil is every AI.
struct BypassSheet: View {
    var who: String?
    var onConfirm: (UInt32) -> Void
    @State private var minutes = AutopilotText.bypassMinutes[0]
    @Environment(\.dismiss) private var dismiss
    @Environment(\.feedback) private var feedback

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 0) {
                IconTile(symbol: "bolt.fill", tint: Palette.danger, size: 48, filled: true)
                Text(who.map { "Bypass \(untrusted($0))?" } ?? "Bypass for every AI?")
                    .font(RFont.sans(22, .semibold))
                    .foregroundStyle(Palette.text)
                    .padding(.top, 14)
                Text("Requests are approved without asking until the time runs out. Approved means done: an email sent cannot be unsent.")
                    .font(RFont.sans(15.5))
                    .foregroundStyle(Palette.secondary)
                    .fixedSize(horizontal: false, vertical: true)
                    .padding(.top, 8)
                HStack(alignment: .top, spacing: 10) {
                    Image(systemName: "checkmark.shield").font(.system(size: 16)).foregroundStyle(Palette.secondary)
                    Text("Still asked every time: new connections, permissions, passwords and secrets, deletions and other one-off changes, SSH and flagged files.")
                        .font(RFont.sans(13.5))
                        .foregroundStyle(Palette.secondary)
                        .fixedSize(horizontal: false, vertical: true)
                }
                .padding(12)
                .frame(maxWidth: .infinity, alignment: .leading)
                .background(Palette.controlFill, in: RoundedRectangle(cornerRadius: 14, style: .continuous))
                .padding(.top, 14)
                Text("For")
                    .font(RFont.sans(13, .medium))
                    .foregroundStyle(Palette.tertiary)
                    .padding(.top, 18)
                    .padding(.bottom, 8)
                HStack(spacing: 8) {
                    ForEach(AutopilotText.bypassMinutes, id: \.self) { m in
                        AutopilotChip(title: AutopilotText.bypassChoice(m), selected: minutes == m, identifier: "bypass:\(m)") { minutes = m }
                    }
                }
                HStack(spacing: 10) {
                    ActionButton(title: "Cancel", kind: .secondary) { dismiss() }
                    Button {
                        feedback.play(.tap)
                        dismiss()
                        onConfirm(minutes)
                    } label: {
                        Label("Turn on", systemImage: "bolt.fill").font(RFont.sans(16, .semibold))
                    }
                    .buttonStyle(CapsuleButtonStyle(kind: .danger, height: 50))
                    .accessibilityIdentifier("confirmBypass")
                }
                .padding(.top, 24)
            }
            .padding(24)
        }
        .scrollBounceBehavior(.basedOnSize)
        .background(Palette.background.ignoresSafeArea())
        .presentationDetents([.medium, .large])
        .presentationDragIndicator(.visible)
        .presentationSizing(.form)
        .onAppear { feedback.cue(.open) }
        .accessibilityIdentifier("bypassDialog")
    }
}

/// Naming a profile and picking its icon (new and rename).
struct ProfileEditorSheet: View {
    var title: String
    var initialName: String
    var initialIcon: String?
    var confirmLabel: String
    var onConfirm: (String, String?) -> Void
    @State private var name = ""
    @State private var icon: String?
    @FocusState private var focused: Bool
    @Environment(\.dismiss) private var dismiss
    @Environment(\.feedback) private var feedback

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            Text(title).font(RFont.sans(22, .semibold)).foregroundStyle(Palette.text)
            InputWell {
                TextField("Name, e.g. Side project", text: $name)
                    .font(RFont.sans(16))
                    .focused($focused)
                    .submitLabel(.done)
                    .onSubmit(save)
                    .onChange(of: name) { if name.count > 40 { name = String(name.prefix(40)) } }
                    .accessibilityIdentifier("profileName")
            }
            .padding(.top, 14)
            Text("Icon")
                .font(RFont.sans(13, .medium))
                .foregroundStyle(Palette.tertiary)
                .padding(.top, 18)
                .padding(.bottom, 8)
            HStack(spacing: 6) {
                ForEach(AutopilotText.icons, id: \.self) { e in
                    let on = e == icon
                    Button {
                        feedback.play(.selection)
                        icon = e
                    } label: {
                        Text(e)
                            .font(.system(size: 19))
                            .frame(width: 38, height: 38)
                            .background(on ? Palette.accentSoft : .clear, in: Circle())
                            .overlay(Circle().strokeBorder(on ? Palette.accent : .clear, lineWidth: 1.5))
                    }
                    .buttonStyle(PressDim())
                    .accessibilityAddTraits(on ? .isSelected : [])
                    .accessibilityIdentifier("icon:\(e)")
                }
            }
            Spacer(minLength: 20)
            HStack(spacing: 10) {
                ActionButton(title: "Cancel", kind: .secondary) { dismiss() }
                ActionButton(title: confirmLabel, kind: .accent, enabled: !trimmed.isEmpty, action: save)
                    .accessibilityIdentifier("saveProfile")
            }
        }
        .padding(24)
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
        .background(Palette.background.ignoresSafeArea())
        .presentationDetents([.height(340), .medium])
        .presentationDragIndicator(.visible)
        .presentationSizing(.form)
        .onAppear {
            name = initialName
            icon = initialIcon
            focused = true
            feedback.cue(.open)
        }
        .accessibilityIdentifier("profileDialog")
    }

    private var trimmed: String { name.trimmingCharacters(in: .whitespacesAndNewlines) }

    private func save() {
        guard !trimmed.isEmpty else { return }
        dismiss()
        onConfirm(trimmed, icon)
    }
}

/// Where a screen in Autopilot opens the next one: from the section's root the detail column (or a push on a
/// phone), from a pushed Autopilot (Activity's pill, Settings) one more screen on top.
extension AppModel {
    func openFromAutopilot(_ route: Route) {
        if section == .autopilot && !path(section).contains(.autopilot) { show(route) } else { push(route) }
    }
}
