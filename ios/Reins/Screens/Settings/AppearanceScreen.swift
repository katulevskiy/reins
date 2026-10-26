import SwiftUI

/// Settings > Appearance (iOS-Settings.png, 04): a small room of your own. The theme (the phone's, Paper or Graphite)
/// as three small phones, and grain on fields. Changes apply live, no restart.
struct AppearanceScreen: View {
    @AppStorage(Appearance.themeKey, store: AppGroup.defaults) private var theme = Appearance.Theme.system.rawValue
    @AppStorage(Appearance.grainKey, store: AppGroup.defaults) private var grain = true
    @Environment(\.feedback) private var feedback

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 0) {
                Text("Appearance")
                    .textRole(.title)
                    .foregroundStyle(Palette.ink)
                    .accessibilityAddTraits(.isHeader)
                    .padding(.horizontal, 24)
                HStack(spacing: 12) {
                    ForEach(Appearance.Theme.allCases, id: \.self) { t in
                        ThemeTile(theme: t, selected: theme == t.rawValue) {
                            guard theme != t.rawValue else { return }
                            feedback.play(.selection)
                            theme = t.rawValue
                        }
                    }
                }
                .padding(.horizontal, 24)
                .padding(.top, 24)
                VStack(spacing: 0) {
                    Toggle(isOn: $grain) {
                        VStack(alignment: .leading, spacing: 1) {
                            Text("Grain on fields").font(TextRole.row.font()).foregroundStyle(Palette.ink)
                            Text("A painted finish on requests").textRole(.meta).foregroundStyle(Palette.inkMeta)
                        }
                    }
                    .toggleStyle(SignalSwitchStyle())
                    .padding(.vertical, 12)
                    .overlay(alignment: .bottom) { Rectangle().fill(Palette.rule).frame(height: 1) }
                    .accessibilityIdentifier("grainSwitch")
                }
                .padding(.horizontal, 24)
                .padding(.top, 26)
            }
            .padding(.bottom, 32)
        }
        .pageBackground()
        .navigationTitle("")
        .navigationBarTitleDisplayMode(.inline)
    }
}

/// One theme as a small phone: a field's band on top and two lines of text, in that theme's paper; the chosen one is
/// ringed in violet.
private struct ThemeTile: View {
    var theme: Appearance.Theme
    var selected: Bool
    var action: () -> Void

    var body: some View {
        Button(action: action) {
            VStack(spacing: 8) {
                phone
                    .frame(height: 112)
                    .padding(3)
                    .overlay(RoundedRectangle(cornerRadius: 17, style: .continuous).strokeBorder(selected ? Palette.violet : .clear, lineWidth: 2))
                Text(theme.title)
                    .font(RFont.sans(13.5, selected ? .semibold : .regular))
                    .foregroundStyle(selected ? Palette.ink : Palette.ink2)
            }
            .frame(maxWidth: .infinity)
            .contentShape(Rectangle())
        }
        .buttonStyle(PressDim())
        .accessibilityLabel(theme.title)
        .accessibilityAddTraits(selected ? [.isButton, .isSelected] : .isButton)
        .accessibilityIdentifier("theme:\(theme.rawValue)")
    }

    /// The phone: half and half for System, else that theme's paper.
    @ViewBuilder private var phone: some View {
        let shape = RoundedRectangle(cornerRadius: 14, style: .continuous)
        ZStack {
            switch theme {
            case .system:
                HStack(spacing: 0) {
                    Color(uiColor: UIColor(hex: 0xF5F5F7))
                    Color(uiColor: UIColor(hex: 0x111114))
                }
            case .paper: Color(uiColor: UIColor(hex: 0xF5F5F7))
            case .graphite: Color(uiColor: UIColor(hex: 0x111114))
            }
            VStack(alignment: .leading, spacing: 6) {
                FieldKind.push.fill.frame(height: 34)
                Capsule().fill(Color.gray.opacity(0.35)).frame(width: 46, height: 4).padding(.horizontal, 8)
                Capsule().fill(Color.gray.opacity(0.25)).frame(width: 30, height: 4).padding(.horizontal, 8)
                Spacer(minLength: 0)
            }
        }
        .clipShape(shape)
        .overlay(shape.strokeBorder(Palette.rule, lineWidth: 1))
    }
}
