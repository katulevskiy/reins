import SwiftUI

/// A tab's header row (the title and its controls) pinned under the status bar while the page scrolls beneath it.
/// Once something scrolls under it, it stands on the shared glass (the blurred art where there is art, the page's paper
/// beyond, the boards' fill) running up under the status bar to the screen's edge, with a soft bottom edge so the
/// rows fade in under it. Solid paper under Reduce Transparency.
struct StickyBackdrop: View {
    var shown: Bool
    @Environment(\.accessibilityReduceTransparency) private var reduceTransparency
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    /// The soft bottom edge.
    static let fade: CGFloat = 14

    var body: some View {
        // Masked before it reaches out: a mask sized after the negative padding would clip it to the header's band.
        Group {
            if reduceTransparency { Palette.paper } else { FrostedGlass() }
        }
        .mask {
            VStack(spacing: 0) {
                Rectangle()
                LinearGradient(colors: [.black, .clear], startPoint: .top, endPoint: .bottom).frame(height: Self.fade)
            }
        }
        .padding(.top, -CompactBanner.reach)
        .padding(.bottom, -Self.fade)
        .opacity(shown ? 1 : 0)
        .animation(reduceMotion ? nil : .easeOut(duration: 0.15), value: shown)
        .allowsHitTesting(false)
        .accessibilityHidden(true)
    }
}

/// Pins `header` above a scroll view, its backdrop shown once the content has scrolled under it. A page that already
/// follows its scroll (one `onScrollGeometryChange` per scroll view is honoured) passes `scrolled` itself.
private struct StickyHeader<Header: View>: ViewModifier {
    var scrolled: Bool?
    var header: Header
    @State private var own = false

    func body(content: Content) -> some View {
        content
            .onScrollGeometryChange(for: Bool.self) { $0.contentOffset.y + $0.contentInsets.top > 1 } action: { _, now in
                if now != own { own = now }
            }
            // Our glass takes the place of the system's blur at the top edge.
            .scrollEdgeEffectHidden(true, for: .top)
            .safeAreaInset(edge: .top, spacing: 0) {
                header
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .background(alignment: .top) { StickyBackdrop(shown: scrolled ?? own) }
            }
    }
}

extension View {
    /// A tab's header that stays put under the status bar while this scroll view scrolls beneath it.
    func stickyHeader<Header: View>(scrolled: Bool? = nil, @ViewBuilder _ header: () -> Header) -> some View {
        modifier(StickyHeader(scrolled: scrolled, header: header()))
    }
}
