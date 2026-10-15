import SwiftUI
import WidgetKit

/// The grain that makes a field read as pigment, not plastic: a grey noise tile (Signal.xcassets `grain`, 360 px @2x,
/// so it repeats every 180 pt) blended soft-light over the field's colour. Only ever on fields; never on paper, text or
/// glyphs. Drawn once, never animated. Off under Reduce Transparency and with Settings > Appearance > Grain on fields.
struct Grain: View {
    @Environment(\.accessibilityReduceTransparency) private var reduceTransparency
    @AppStorage(Appearance.grainKey, store: AppGroup.defaults) private var enabled = true

    var body: some View {
        if enabled, !reduceTransparency {
            Image("grain")
                .resizable(resizingMode: .tile)
                .blendMode(.softLight)
                .allowsHitTesting(false)
                .accessibilityHidden(true)
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
