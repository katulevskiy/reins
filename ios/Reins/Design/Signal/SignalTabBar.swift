import SwiftUI

/// The phone's tab bar (Components.png, Tab bars): a floating `inverse` capsule; the chosen tab is a `paper` pill with
/// its glyph and name, the others only their glyph at 62 %. The pill slides to a new tab on a 240 ms spring with a
/// slight overshoot (a cross-fade under Reduce Motion), with the select haptic. A running bypass breathes a dot on its tab.
struct SignalTabBar<Tab: Hashable>: View {
    struct Item: Identifiable {
        var tab: Tab
        var title: String
        var glyph: Glyph
        /// The UI-test identifier ("tab:activity").
        var identifier: String
        var badge = false

        var id: String { identifier }
    }

    var items: [Item]
    @Binding var selection: Tab

    @Namespace private var pill
    @Environment(\.feedback) private var feedback
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    var body: some View {
        HStack(spacing: 2) {
            ForEach(items) { item in
                let chosen = item.tab == selection
                Button {
                    guard !chosen else { return }
                    withAnimation(Motion.of(Motion.tabSpring, reduce: reduceMotion)) { selection = item.tab }
                    feedback.play(.selection)
                } label: {
                    HStack(spacing: 7) {
                        GlyphIcon(item.glyph, size: 20)
                            .overlay(alignment: .topTrailing) {
                                if item.badge { BreathingDot(color: Palette.fieldDelete, size: 7).offset(x: 3, y: -2) }
                            }
                        if chosen {
                            Text(item.title).font(RFont.sans(14.5, .semibold)).lineLimit(1).fixedSize()
                        }
                    }
                    .foregroundStyle(chosen ? Palette.ink : Palette.onInverse)
                    .opacity(chosen ? 1 : 0.62)
                    .padding(.horizontal, chosen ? 16 : 0)
                    .frame(minWidth: 50, minHeight: 44)
                    .background {
                        if chosen {
                            Capsule().fill(Palette.paper).matchedGeometryEffect(id: "pill", in: pill)
                        }
                    }
                    .contentShape(Capsule())
                }
                .buttonStyle(.plain)
                .accessibilityLabel(item.title)
                .accessibilityAddTraits(chosen ? [.isSelected, .isButton] : .isButton)
                .accessibilityIdentifier(item.identifier)
            }
        }
        .padding(6)
        .background(Palette.inverse, in: Capsule())
        // The only thing that floats: 0 14 30 -12 rgba(0,0,0,.45), without the negative spread SwiftUI can't draw.
        .shadow(color: .black.opacity(0.3), radius: 12, y: 14)
        .accessibilityElement(children: .contain)
    }
}
