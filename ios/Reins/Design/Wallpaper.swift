import SwiftUI
import UIKit

/// The art behind Activity (the Wallpaper and Settings boards): dithered and treated images, faded into the page at the
/// bottom, one for dark and one for light (Halo and Torii dither by default), or the user's own photo with an effect.
/// Groups over it become frosted panes; Reduce Transparency makes the panes solid and keeps the art.
enum Wallpaper: String, CaseIterable, Identifiable {
    case halo, crescent, figure, hands, ruins, eyes, torii, toriiDither = "torii-dither"
    /// The user's own photo, treated (`OwnWallpaper`).
    case own

    var id: String { rawValue }

    static let shipped: [Wallpaper] = [.halo, .crescent, .figure, .hands, .ruins, .eyes, .torii, .toriiDither]

    var title: String {
        switch self {
        case .halo: "Halo"
        case .crescent: "Crescent"
        case .figure: "Figure"
        case .hands: "Hands"
        case .ruins: "Ruins"
        case .eyes: "Eyes"
        case .torii: "Torii"
        case .toriiDither: "Torii dither"
        case .own: "Your photo"
        }
    }

    var assetName: String { "wallpaper-\(rawValue)" }

    static func key(_ scheme: ColorScheme) -> String { scheme == .dark ? "wallpaper.dark" : "wallpaper.light" }

    static func defaultFor(_ scheme: ColorScheme) -> Wallpaper { scheme == .dark ? .halo : .toriiDither }

    /// The image to draw, or nil when the own photo is chosen but missing.
    func image(_ scheme: ColorScheme) -> UIImage? {
        self == .own ? OwnWallpaper.load(scheme) : UIImage(named: assetName)
    }
}

/// The user's own wallpaper, kept as a treated JPEG in the app's container, one per theme.
enum OwnWallpaper {
    static func url(_ scheme: ColorScheme) -> URL {
        let dir = (try? FileManager.default.url(for: .applicationSupportDirectory, in: .userDomainMask, appropriateFor: nil, create: true))
            ?? FileManager.default.temporaryDirectory
        return dir.appendingPathComponent(scheme == .dark ? "wallpaper-dark.jpg" : "wallpaper-light.jpg")
    }

    static func load(_ scheme: ColorScheme) -> UIImage? {
        UIImage(contentsOfFile: url(scheme).path)
    }

    static func save(_ image: UIImage, _ scheme: ColorScheme) throws {
        guard let data = image.jpegData(compressionQuality: 0.86) else { return }
        try data.write(to: url(scheme), options: .atomic)
    }
}

/// The wallpaper layer at the top of a page: the image across the full width (its 780 × 1000 art is 390 × 500 at the
/// boards' size), already faded into the page at its bottom, and a shade under the status bar so the title reads.
struct WallpaperLayer: View {
    @Environment(\.colorScheme) private var scheme
    @AppStorage(Wallpaper.key(.dark), store: AppGroup.defaults) private var dark = Wallpaper.halo.rawValue
    @AppStorage(Wallpaper.key(.light), store: AppGroup.defaults) private var light = Wallpaper.toriiDither.rawValue

    var body: some View {
        let chosen = Wallpaper(rawValue: scheme == .dark ? dark : light) ?? Wallpaper.defaultFor(scheme)
        GeometryReader { geo in
            ZStack(alignment: .top) {
                if let image = chosen.image(scheme) ?? Wallpaper.defaultFor(scheme).image(scheme) {
                    Image(uiImage: image)
                        .resizable()
                        .interpolation(.medium)
                        .scaledToFill()
                        .frame(width: geo.size.width, height: geo.size.width * 500 / 390, alignment: .top)
                        .clipped()
                }
                LinearGradient(
                    colors: [scheme == .dark ? .black.opacity(0.55) : Color(red: 246 / 255, green: 246 / 255, blue: 248 / 255).opacity(0.75), .clear],
                    startPoint: .top, endPoint: .bottom
                )
                .frame(height: 150)
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .top)
        }
        .ignoresSafeArea()
        .allowsHitTesting(false)
        .accessibilityHidden(true)
    }
}

extension EnvironmentValues {
    /// The page draws a wallpaper: groups become frosted panes.
    @Entry var onWallpaper = false
}

/// A frosted pane (the Wallpaper board's readable glass, `pf2`): heavy blur with the art's colour coming through, a
/// little fill, a lit top edge and a soft shadow. Solid raised paper with a hairline ring under Reduce Transparency.
struct FrostedPane: View {
    var radius: CGFloat = 20
    @Environment(\.colorScheme) private var scheme
    @Environment(\.accessibilityReduceTransparency) private var reduceTransparency

    var body: some View {
        let shape = RoundedRectangle(cornerRadius: radius, style: .continuous)
        if reduceTransparency {
            shape.fill(Palette.paperRaised)
                .overlay(shape.strokeBorder(Palette.rule, lineWidth: 1))
        } else {
            let dark = scheme == .dark
            shape.fill(.ultraThinMaterial)
                .overlay(shape.fill(dark ? Color(red: 30 / 255, green: 30 / 255, blue: 36 / 255).opacity(0.4) : Color.white.opacity(0.4)))
                .overlay(shape.strokeBorder(Color.white.opacity(dark ? 0.08 : 0.4), lineWidth: 1))
                .overlay(alignment: .top) {
                    // The lit top edge.
                    shape.strokeBorder(
                        LinearGradient(colors: [Color.white.opacity(dark ? 0.16 : 0.8), .clear], startPoint: .top, endPoint: .init(x: 0.5, y: 0.08)),
                        lineWidth: 1
                    )
                }
                .shadow(color: .black.opacity(dark ? 0.3 : 0.06), radius: 15, y: 8)
        }
    }
}
