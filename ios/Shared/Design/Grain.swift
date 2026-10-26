import SwiftUI
import WidgetKit

/// The finish on every colour surface (fields, waiting cards, mode cards, big tiles): Next's Aurora, light pooling in
/// the top-right corner and shade in the bottom-left over a grey noise tile (Signal.xcassets `grain`, 360 px @2x, so it
/// repeats every 180 pt), all blended soft-light over the colour. Never on paper, text or glyphs. Drawn once, never
/// animated. Off (flat colour) under Reduce Transparency and with Settings > Appearance > Grain on fields.
struct Grain: View {
    @Environment(\.accessibilityReduceTransparency) private var reduceTransparency
    @AppStorage(Appearance.grainKey, store: AppGroup.defaults) private var enabled = true

    var body: some View {
        if enabled, !reduceTransparency {
            // As the boards stack them: the noise at the bottom, the pools over it, all soft-light on the colour.
            ZStack {
                Image("grain").resizable(resizingMode: .tile)
                AuroraLight()
            }
            .compositingGroup()
            .blendMode(.softLight)
            .allowsHitTesting(false)
            .accessibilityHidden(true)
        }
    }
}

/// Aurora's three pools (the boards' `tx-aurora`): white from the top-right corner, shade from the bottom-left, a warm
/// glow at the upper left. Each is an ellipse sized to the surface, as CSS draws `radial-gradient(70% 90% at 92% 0%)`.
struct AuroraLight: View {
    private struct Pool {
        var center: UnitPoint
        var radii: CGSize
        var color: Color
        var reach: CGFloat
    }

    private static let pools = [
        Pool(center: UnitPoint(x: 0.92, y: 0), radii: CGSize(width: 0.7, height: 0.9), color: .white.opacity(0.7), reach: 0.6),
        Pool(center: UnitPoint(x: 0, y: 1), radii: CGSize(width: 0.6, height: 0.8), color: .black.opacity(0.45), reach: 0.7),
        Pool(center: UnitPoint(x: 0.3, y: 0.2), radii: CGSize(width: 0.5, height: 0.6), color: Color(red: 1, green: 220 / 255, blue: 180 / 255).opacity(0.35), reach: 0.7),
    ]

    var body: some View {
        Canvas { context, size in
            guard size.width > 0, size.height > 0 else { return }
            for pool in Self.pools {
                let rx = pool.radii.width * size.width, ry = pool.radii.height * size.height
                var layer = context
                layer.translateBy(x: pool.center.x * size.width, y: pool.center.y * size.height)
                layer.scaleBy(x: rx / ry, y: 1)
                let gradient = Gradient(stops: [.init(color: pool.color, location: 0), .init(color: pool.color.opacity(0), location: pool.reach)])
                layer.fill(
                    Path(CGRect(x: -4 * ry, y: -4 * ry, width: 8 * ry, height: 8 * ry)),
                    with: .radialGradient(gradient, center: .zero, startRadius: 0, endRadius: ry)
                )
            }
        }
    }
}

/// The grain where nothing can blend (widgets, Live Activities): specks at about 6 % with a normal blend, baked from
/// the same texture. Only in full colour; a tinted or vibrant Home Screen and the Dynamic Island stay plain.
struct WidgetGrain: View {
    @Environment(\.widgetRenderingMode) private var renderingMode
    @Environment(\.accessibilityReduceTransparency) private var reduceTransparency
    @AppStorage(Appearance.grainKey, store: AppGroup.defaults) private var enabled = true

    var body: some View {
        if enabled, !reduceTransparency, renderingMode == .fullColor {
            Image("grain-widget")
                .resizable(resizingMode: .tile)
                .allowsHitTesting(false)
                .accessibilityHidden(true)
        }
    }
}
