import SwiftUI

/// Settings > Appearance (the Settings board, 04), quiet: the theme (the phone's, Paper or Graphite) as three small
/// phones, the focal point, and grain on fields. Changes apply at once, widgets too.
struct AppearanceScreen: View {
    @AppStorage(Appearance.themeKey, store: AppGroup.defaults) private var theme = Appearance.Theme.system.rawValue
    @AppStorage(Appearance.grainKey, store: AppGroup.defaults) private var grain = true
    @Environment(\.feedback) private var feedback

    var body: some View {
        QuietPage("Appearance") {
            SectionHeader("Theme", quiet: true)
            HStack(alignment: .top, spacing: 10) {
                ForEach(Appearance.Theme.allCases, id: \.self) { t in
                    ThemeTile(theme: t, selected: theme == t.rawValue) {
                        guard theme != t.rawValue else { return }
                        feedback.play(.selection)
                        theme = t.rawValue
                    }
                }
            }
            .padding(.horizontal, 20)
            .padding(.top, 6)
            SectionHeader("Look", quiet: true)
                .padding(.top, 4)
            FlatGroup {
                SwitchTileRow(title: "Grain on fields", glyph: .appearance, isOn: grain, id: "grainSwitch") { on in
                    grain = on
                }
            }
        }
    }
}

/// One theme as a small phone: a field's band (with its grain) on top and three lines of text, in that theme's paper;
/// the chosen one is ringed in violet.
private struct ThemeTile: View {
    var theme: Appearance.Theme
    var selected: Bool
    var action: () -> Void

    var body: some View {
        Button(action: action) {
            VStack(spacing: 8) {
                phone
                    .padding(3)
                    .overlay(RoundedRectangle(cornerRadius: 19, style: .continuous).strokeBorder(selected ? Palette.violet : .clear, lineWidth: 2))
                Text(theme.title)
                    .font(RFont.sans(14, selected ? .semibold : .regular))
                    .foregroundStyle(selected ? Palette.ink : Palette.ink2)
                    .lineLimit(1)
                    .minimumScaleFactor(0.7)
            }
            .frame(maxWidth: .infinity)
            .contentShape(Rectangle())
        }
        .buttonStyle(PressDimsStyle())
        .accessibilityLabel(theme.title)
        .accessibilityAddTraits(selected ? [.isButton, .isSelected] : .isButton)
        .accessibilityIdentifier("theme:\(theme.rawValue)")
    }

    private static let paper = Color(uiColor: UIColor(hex: 0xF5F5F7))
    private static let graphite = Color(uiColor: UIColor(hex: 0x111114))

    /// The phone: half and half for System, else that theme's paper.
    private var phone: some View {
        let shape = RoundedRectangle(cornerRadius: 16, style: .continuous)
        return ZStack(alignment: .top) {
            switch theme {
            case .system: HStack(spacing: 0) { Self.paper; Self.graphite }
            case .paper: Self.paper
            case .graphite: Self.graphite
            }
            VStack(alignment: .leading, spacing: 0) {
                FieldSurface(kind: .push).frame(height: 46)
                bar(50, 0.4).padding(.top, 10)
                bar(34, 0.28).padding(.top, 7)
                bar(44, 0.28).padding(.top, 7)
            }
        }
        .frame(height: 150)
        .clipShape(shape)
        .overlay(shape.strokeBorder(Palette.rule, lineWidth: 1))
    }

    private func bar(_ width: CGFloat, _ opacity: Double) -> some View {
        Capsule().fill(Color(white: 0.5).opacity(opacity)).frame(width: width, height: 5).padding(.horizontal, 10)
    }
}
