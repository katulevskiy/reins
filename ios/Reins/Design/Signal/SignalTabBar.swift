import SwiftUI

/// The phone's tab bar (the System board): a floating `inverse` capsule; the chosen tab is a `paper` pill with its glyph
/// and name, the others only their glyph at 70 %. The pill slides to a new tab on a 240 ms spring with a slight
/// overshoot (a cross-fade under Reduce Motion), with the select haptic. A running Overnight, Lockdown or Bypass puts a
/// dot in its colour on Trust.
struct SignalTabBar<Tab: Hashable>: View {
    struct Item: Identifiable {
        var tab: Tab
        var title: String
        var glyph: Glyph
        /// The UI-test identifier ("tab:activity").
        var identifier: String
        /// A timed mode runs: a dot in its colour on the tab.
        var dot: Color? = nil
        /// How many wait (Activity): a count on the glyph, "99+" past 99.
        var count = 0

        var id: String { identifier }
    }

    var items: [Item]
    @Binding var selection: Tab

    /// Its height before it has been measured: a 44 pt tab in 6 pt of capsule.
    static var estimatedHeight: CGFloat { 56 }

    @Namespace private var pill
    @Environment(\.feedback) private var feedback
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @Environment(\.dynamicTypeSize) private var typeSize

    var body: some View {
        // At accessibility sizes (the Largest text board) the bar keeps its size and its icons; the chosen tab drops its
        // name, and holding a tab shows it in the Large Content Viewer.
        let large = typeSize.isAccessibilitySize
        HStack(spacing: 2) {
            ForEach(items) { item in
                let chosen = item.tab == selection
                Button {
                    // The chosen tab again: back to its root, as a system tab bar does.
                    guard !chosen else { return selection = item.tab }
                    withAnimation(Motion.of(Motion.tabSpring, reduce: reduceMotion)) { selection = item.tab }
                    feedback.play(.selection)
                } label: {
                    HStack(spacing: 7) {
                        GlyphIcon(item.glyph, size: 20)
                            .opacity(chosen ? 1 : 0.7)
                            .overlay(alignment: .topTrailing) {
                                if !chosen, item.count > 0 {
                                    CountBadge(count: item.count, tint: Palette.fieldDelete)
                                        .scaleEffect(0.85)
                                        .offset(x: 11, y: -9)
                                }
                            }
                        if chosen, !large {
                            Text(item.title).font(RFont.sans(14.5, .semibold)).lineLimit(1).fixedSize()
                        }
                    }
                    .foregroundStyle(chosen ? Palette.ink : Palette.onInverse)
                    .padding(.horizontal, chosen && !large ? 15 : 0)
                    .frame(minWidth: 50, minHeight: 44)
                    .background {
                        if chosen {
                            Capsule().fill(Palette.paper).matchedGeometryEffect(id: "pill", in: pill)
                        }
                    }
                    .overlay(alignment: .topTrailing) {
                        if let dot = item.dot {
                            Circle().fill(dot).frame(width: 7, height: 7)
                                .padding(2).background(chosen ? Palette.paper : Palette.inverse, in: Circle())
                                .padding(.top, 7).padding(.trailing, 7)
                        }
                    }
                    .contentShape(Capsule())
                }
                .buttonStyle(.plain)
                .accessibilityShowsLargeContentViewer {
                    Label { Text(item.title) } icon: { GlyphIcon(item.glyph, size: 20) }
                }
                .accessibilityLabel(item.title)
                .accessibilityValue(item.count > 0 ? "\(item.count) waiting" : item.dot != nil ? "A mode is running" : "")
                .accessibilityAddTraits(chosen ? [.isSelected, .isButton] : .isButton)
                .accessibilityIdentifier(item.identifier)
            }
        }
        .padding(6)
        .dynamicTypeSize(...DynamicTypeSize.large)
        .background(Palette.inverse, in: Capsule())
        // The only thing that floats: 0 14 30 -12 rgba(0,0,0,.45), without the negative spread SwiftUI can't draw.
        .shadow(color: .black.opacity(0.3), radius: 12, y: 14)
        .accessibilityElement(children: .contain)
    }
}

extension View {
    /// Room for the floating tab bar under a root page: its scrolling content (and anything that keeps to the safe
    /// area, as an empty state or the "Latest" button) ends `bar` plus a small margin above the bottom, so the last
    /// row sits fully above the bar when scrolled to the end. `bar` is the bar's height and its gap to the safe area.
    func tabBarClearance(_ bar: CGFloat) -> some View {
        safeAreaPadding(.bottom, bar + TabBarClearance.margin)
    }
}

enum TabBarClearance {
    /// Between the last row and the top of the tab bar.
    static let margin: CGFloat = 12
}
