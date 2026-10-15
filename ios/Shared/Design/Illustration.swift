import SwiftUI

/// Signal's two ink drawings (Signal.xcassets, light and dark): the pond for an empty or all-quiet screen, the kite for
/// arriving. Decoration: hidden from VoiceOver, which reads the mood line beside it instead.
struct Illustration: View {
    enum Name: String, CaseIterable {
        /// viewBox 320 × 170.
        case pond
        /// viewBox 320 × 240.
        case kite

        var aspect: CGFloat { self == .pond ? 320 / 170 : 320 / 240 }
    }

    var name: Name

    init(_ name: Name) {
        self.name = name
    }

    var body: some View {
        Image(name.rawValue)
            .resizable()
            .aspectRatio(name.aspect, contentMode: .fit)
            .accessibilityHidden(true)
    }
}

extension Image {
    /// A Signal glyph as a template image: it takes the foreground style. It is an image, so `.font` does not size it:
    /// use `GlyphIcon`, or `.resizable().frame(…)`.
    init(glyph: Glyph) {
        self.init(glyph.rawValue)
    }
}

/// A glyph at a point size (14 chevrons, 16–19 buttons and verdicts, 20–22 rows and tiles), in the foreground style.
struct GlyphIcon: View {
    var glyph: Glyph
    var size: CGFloat = 20

    init(_ glyph: Glyph, size: CGFloat = 20) {
        self.glyph = glyph
        self.size = size
    }

    @ScaledMetric(relativeTo: .body) private var scale: CGFloat = 1

    var body: some View {
        Image(glyph: glyph)
            .resizable()
            .scaledToFit()
            .frame(width: size * scale, height: size * scale)
            .accessibilityHidden(true)
    }
}
