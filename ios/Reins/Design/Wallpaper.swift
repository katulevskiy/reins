import CoreImage
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
    /// The page's scroll, when the art scrolls away with it.
    var scroll: ScrollTracker? = nil
    @Environment(\.colorScheme) private var scheme
    @AppStorage(Wallpaper.key(.dark), store: AppGroup.defaults) private var dark = Wallpaper.halo.rawValue
    @AppStorage(Wallpaper.key(.light), store: AppGroup.defaults) private var light = Wallpaper.toriiDither.rawValue

    var body: some View {
        let chosen = Wallpaper(rawValue: scheme == .dark ? dark : light) ?? Wallpaper.defaultFor(scheme)
        GeometryReader { geo in
            ZStack(alignment: .top) {
                let shade = scheme == .dark ? Color.black.opacity(0.55) : Color(red: 246 / 255, green: 246 / 255, blue: 248 / 255).opacity(0.75)
                let shown = chosen.image(scheme) != nil ? chosen : Wallpaper.defaultFor(scheme)
                if let image = shown.image(scheme) {
                    Image(uiImage: image)
                        .resizable()
                        .interpolation(.medium)
                        .scaledToFill()
                        .frame(width: geo.size.width, height: geo.size.width * 500 / 390, alignment: .top)
                        .clipped()
                        .backdropArt(key: "\(shown.rawValue)-\(scheme == .dark ? "dark" : "light")-\(Self.ownStamp(shown, scheme))", image: image, dark: scheme == .dark, shade: shade)
                }
                LinearGradient(colors: [shade, .clear], startPoint: .top, endPoint: .bottom)
                    .frame(height: 150)
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .top)
            .offset(y: -max(scroll?.y ?? 0, 0))
        }
        .ignoresSafeArea()
        .allowsHitTesting(false)
        .accessibilityHidden(true)
    }
}

extension WallpaperLayer {
    /// The own photo's file date, so a new photo blurs anew under the same name.
    static func ownStamp(_ wallpaper: Wallpaper, _ scheme: ColorScheme) -> String {
        guard wallpaper == .own else { return "" }
        let date = (try? FileManager.default.attributesOfItem(atPath: OwnWallpaper.url(scheme).path)[.modificationDate]) as? Date
        return String(date?.timeIntervalSince1970 ?? 0)
    }
}

extension EnvironmentValues {
    /// The page draws a wallpaper: groups become frosted panes.
    @Entry var onWallpaper = false
}

/// The art under the page's frosted glass and where it is on screen. The art registers itself (`backdropArt`); every
/// pane and every row slice of a group then draws the same blurred copy of it at the art's place in screen
/// coordinates, offset by its own frame, so neighbouring panes line up exactly whatever the scroll or the safe area.
@Observable
@MainActor
final class Backdrop {
    static let shared = Backdrop()

    /// The blurred art (saturated and lifted or dimmed for the theme, as the boards' `pf2` glass filters it).
    private(set) var blurred: UIImage?
    /// The art's drawn rect, in global coordinates.
    private(set) var frame: CGRect = .zero
    /// The top shade drawn over the art (the title's legibility), drawn over the blurred copy too.
    private(set) var shade: Color = .clear
    private var key = ""

    func register(key: String, art: UIImage, dark: Bool, frame: CGRect, shade: Color) {
        if key != self.key {
            self.key = key
            blurred = Self.blur(art, dark: dark)
        }
        if frame != self.frame { self.frame = frame }
        self.shade = shade
    }

    private static let context = CIContext(options: [.useSoftwareRenderer: false])

    /// A quarter-size copy blurred as the boards' glass does (40 pt at the art's size), saturated 1.9, a touch lighter
    /// in light and darker in dark. The edges are extended before the blur, so the copy never darkens at its sides.
    static func blur(_ art: UIImage, dark: Bool) -> UIImage? {
        guard let cg = art.cgImage else { return nil }
        let source = CIImage(cgImage: cg)
        let small = source.transformed(by: CGAffineTransform(scaleX: 0.25, y: 0.25))
        let extent = small.extent
        let blurred = small.clampedToExtent().applyingGaussianBlur(sigma: 20).cropped(to: extent)
        let tuned = blurred.applyingFilter("CIColorControls", parameters: [
            kCIInputSaturationKey: 1.9, kCIInputBrightnessKey: dark ? -0.1 : 0.04,
        ])
        guard let out = context.createCGImage(tuned, from: extent) else { return nil }
        return UIImage(cgImage: out)
    }

    /// Where an image drawn aspect-filled into `box`, centred across and from the top, actually lies.
    nonisolated static func fillRect(image: CGSize, in box: CGRect) -> CGRect {
        guard image.width > 0, image.height > 0 else { return box }
        let scale = max(box.width / image.width, box.height / image.height)
        let size = CGSize(width: image.width * scale, height: image.height * scale)
        return CGRect(x: box.midX - size.width / 2, y: box.minY, width: size.width, height: size.height)
    }
}

extension View {
    /// Registers this view (an image drawn aspect-filled into its frame, from the top) as the art under the page's
    /// glass, tracking where it is on screen as it moves.
    func backdropArt(key: String, image: UIImage, dark: Bool, shade: Color = .clear) -> some View {
        onGeometryChange(for: CGRect.self) { $0.frame(in: .global) } action: { box in
            Backdrop.shared.register(key: key, art: image, dark: dark, frame: Backdrop.fillRect(image: image.size, in: box), shade: shade)
        }
    }
}

/// The glass itself, for any shape (a pane, a row's slice of a group, the sticky header): the shared blurred art at
/// its place on screen, the page's paper beyond it, the top shade, then the boards' fill. Clip it to the shape.
struct FrostedGlass: View {
    @Environment(\.colorScheme) private var scheme

    var body: some View {
        let dark = scheme == .dark
        let backdrop = Backdrop.shared
        GeometryReader { geo in
            let me = geo.frame(in: .global)
            let art = backdrop.frame
            ZStack(alignment: .topLeading) {
                Palette.paper
                if let blurred = backdrop.blurred, art.width > 0 {
                    Image(uiImage: blurred)
                        .resizable()
                        .interpolation(.medium)
                        .frame(width: art.width, height: art.height)
                        .offset(x: art.minX - me.minX, y: art.minY - me.minY)
                    LinearGradient(colors: [backdrop.shade, .clear], startPoint: .top, endPoint: .bottom)
                        .frame(width: art.width, height: 150)
                        .offset(x: art.minX - me.minX, y: art.minY - me.minY)
                }
                (dark ? Color(red: 30 / 255, green: 30 / 255, blue: 36 / 255).opacity(0.4) : Color.white.opacity(0.4))
            }
            .frame(width: geo.size.width, height: geo.size.height, alignment: .topLeading)
            .clipped()
        }
        .allowsHitTesting(false)
        .accessibilityHidden(true)
    }
}

/// A frosted pane (the Wallpaper board's readable glass, `pf2`): the shared glass, a lit top edge, a faint ring and a
/// soft shadow. Solid raised paper with a hairline ring under Reduce Transparency.
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
            FrostedGlass()
                .clipShape(shape)
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
