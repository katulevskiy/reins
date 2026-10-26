import SwiftUI

/// The mode picker on Trust (the Mode picker board, final): Manual or Auto as the 76 pt switch, then one "For a while"
/// row that opens the iOS menu of the three timed modes. Overnight and Lockdown start on tap; Bypass asks how long
/// first. At accessibility text sizes (the Largest text board) Manual and Auto become two full rows in one raised
/// group, the chosen one filled with a check.
struct TrustModePicker: View {
    var settings: AutopilotSettings
    var autoReady: Bool
    var onPick: (AutopilotMode) -> Void
    var onOvernight: () -> Void
    var onLockdown: () -> Void
    var onBypass: () -> Void

    @Environment(\.dynamicTypeSize) private var typeSize
    @Environment(\.feedback) private var feedback

    private var selected: ModeKind { settings.mode == .auto ? .auto : .manual }

    var body: some View {
        VStack(spacing: 14) {
            if typeSize.isAccessibilitySize {
                rows
            } else {
                ModeSwitch(selected: selected, autoReady: autoReady) { kind in
                    onPick(kind == .auto ? .auto : .manual)
                }
            }
            forAWhile
        }
    }

    /// Manual and Auto as two rows (Largest text).
    private var rows: some View {
        VStack(spacing: 0) {
            ForEach([ModeKind.manual, .auto], id: \.self) { kind in
                let on = selected == kind
                Button {
                    if !on { onPick(kind == .auto ? .auto : .manual) }
                } label: {
                    HStack(spacing: 14) {
                        ModeGlyph(kind, size: 20)
                        Text(kind.name).font(RFont.sans(19, .semibold)).frame(maxWidth: .infinity, alignment: .leading)
                        if on { GlyphIcon(.check, size: 18) }
                    }
                    .foregroundStyle(on ? (kind == .auto ? Color.white : Palette.onInverse) : Palette.ink)
                    .padding(.horizontal, 18)
                    .padding(.vertical, 14)
                    .frame(minHeight: 64)
                    .background {
                        if on {
                            ZStack { kind == .auto ? Palette.violetFill : Palette.inverse; Grain() }
                                .clipShape(RoundedRectangle(cornerRadius: 16, style: .continuous))
                                .padding(4)
                        }
                    }
                    .contentShape(Rectangle())
                }
                .buttonStyle(PressDimsStyle())
                .disabled(kind == .auto && !autoReady && !on)
                .accessibilityLabel(kind.name)
                .accessibilityAddTraits(on ? .isSelected : [])
                .accessibilityIdentifier("modeSwitch:\(kind.rawValue)")
            }
        }
        .background(Palette.paperRaised, in: RoundedRectangle(cornerRadius: 20, style: .continuous))
        .overlay(RoundedRectangle(cornerRadius: 20, style: .continuous).strokeBorder(Palette.rule, lineWidth: 1))
        .padding(.horizontal, 16)
    }

    /// The one row; its menu holds Overnight, Lockdown and Bypass (in red).
    private var forAWhile: some View {
        RaisedGroup {
            Menu {
                Button(action: onOvernight) { Label { Text(ModeKind.overnight.name) } icon: { Image(glyph: .moon) } }
                    .accessibilityIdentifier("mode:overnight")
                Button(action: onLockdown) { Label { Text(ModeKind.lockdown.name) } icon: { Image(glyph: .block) } }
                    .accessibilityIdentifier("mode:lockdown")
                Button(role: .destructive, action: onBypass) { Label { Text(ModeKind.bypass.name) } icon: { Image(glyph: .shieldAlert) } }
                    .accessibilityIdentifier("mode:bypass")
            } label: {
                ForAWhileRow()
            }
            .buttonStyle(.plain)
            .simultaneousGesture(TapGesture().onEnded { feedback.play(.tap) })
            .accessibilityLabel("For a while")
            .accessibilityHint("Overnight, Lockdown or Bypass")
            .accessibilityIdentifier("forAWhile")
        }
    }
}
