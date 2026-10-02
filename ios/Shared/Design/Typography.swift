import SwiftUI

/// Type: Geist for the interface, Geist Mono for addresses, codes and numbers (the Android app's `RType`). Sizes
/// follow Dynamic Type relative to the text style closest to them.
enum RFont {
    enum Weight {
        case regular, medium, semibold, bold

        var sansName: String {
            switch self {
            case .regular: "Geist-Regular"
            case .medium: "Geist-Medium"
            case .semibold: "Geist-SemiBold"
            case .bold: "Geist-Bold"
            }
        }

        var monoName: String {
            switch self {
            case .regular: "GeistMono-Regular"
            case .medium: "GeistMono-Medium"
            case .semibold, .bold: "GeistMono-SemiBold"
            }
        }
    }

    static func sans(_ size: CGFloat, _ weight: Weight = .regular) -> Font {
        .custom(weight.sansName, size: size, relativeTo: style(for: size))
    }

    static func mono(_ size: CGFloat, _ weight: Weight = .regular) -> Font {
        .custom(weight.monoName, size: size, relativeTo: style(for: size))
    }

    /// The same face at a fixed size (widgets, Live Activities, fixed-height chrome).
    static func fixedSans(_ size: CGFloat, _ weight: Weight = .regular) -> Font {
        .custom(weight.sansName, fixedSize: size)
    }

    static func fixedMono(_ size: CGFloat, _ weight: Weight = .regular) -> Font {
        .custom(weight.monoName, fixedSize: size)
    }

    private static func style(for size: CGFloat) -> Font.TextStyle {
        switch size {
        case ..<12: .caption2
        case ..<13: .caption
        case ..<15: .footnote
        case ..<16: .subheadline
        case ..<18: .body
        case ..<21: .title3
        case ..<25: .title2
        case ..<31: .title
        default: .largeTitle
        }
    }
}
