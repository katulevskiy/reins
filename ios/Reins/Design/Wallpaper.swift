import CoreImage
import SwiftUI
import UIKit

/// The art behind Activity (the Wallpaper and Settings boards): dithered and treated images, faded into the page at the
/// bottom, one for dark and one for light (Halo and Torii dither by default), or the user's own photo. Which image is
/// one of a theme's three settings (`WallpaperChoice`); the effect and the fade are the others. Groups over it become
/// frosted panes; Reduce Transparency makes the panes solid and keeps the art.
enum Wallpaper: String, CaseIterable, Identifiable {
    case halo, crescent, figure, hands, ruins, eyes, torii, toriiDither = "torii-dither"
    /// The user's own photo (`OwnWallpaper`).
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

    /// Where the image alone was kept before the effect and the fade were settings of their own; read once, by
    /// `WallpaperChoice.migrate`.
    static func legacyKey(_ scheme: ColorScheme) -> String { scheme == .dark ? "wallpaper.dark" : "wallpaper.light" }

    static func defaultFor(_ scheme: ColorScheme) -> Wallpaper { scheme == .dark ? .halo : .toriiDither }

    /// The image itself, before any effect (your photo as it was picked), or nil when your photo is chosen but missing.
    func image(_ scheme: ColorScheme) -> UIImage? {
        self == .own ? OwnWallpaper.load(scheme) : UIImage(named: assetName)
    }
}

/// A theme's wallpaper as three separate settings: the image (a shipped one or your photo), the effect over it and how
/// far it fades into the page. Any image takes any effect. Kept as `image|effect|fade` (the fade in hundredths) under
/// `key(scheme)` in the app group, where `WallpaperLayer` reads it.
struct WallpaperChoice: Hashable, RawRepresentable {
    var image: Wallpaper
    var effect: WallpaperEffect
    /// 0...1, in hundredths. 0 adds none: a shipped image keeps the fade it was drawn with.
    var fade: Double {
        didSet { fade = Self.step(fade) }
    }

    init(image: Wallpaper, effect: WallpaperEffect = .original, fade: Double = 0) {
        self.image = image
        self.effect = effect
        self.fade = Self.step(fade)
    }

    init?(rawValue: String) {
        let parts = rawValue.split(separator: "|", omittingEmptySubsequences: false).map(String.init)
        guard parts.count == 3, let image = Wallpaper(rawValue: parts[0]), let effect = WallpaperEffect(rawValue: parts[1]),
              let hundredths = Int(parts[2]) else { return nil }
        self.init(image: image, effect: effect, fade: Double(hundredths) / 100)
    }

    var rawValue: String { "\(image.rawValue)|\(effect.rawValue)|\(hundredths)" }

    var hundredths: Int { Int((fade * 100).rounded()) }

    static func step(_ fade: Double) -> Double { (min(1, max(0, fade)) * 100).rounded() / 100 }

    static func key(_ scheme: ColorScheme) -> String { scheme == .dark ? "wallpaper2.dark" : "wallpaper2.light" }

    /// The theme's shipped default as it was drawn: no effect, no added fade.
    static func defaultFor(_ scheme: ColorScheme) -> WallpaperChoice { WallpaperChoice(image: .defaultFor(scheme)) }

    /// What a theme draws: the saved choice; before one was saved, the image the old key kept (your photo as it was
    /// baked, so Original and no added fade), or the default.
    static func initial(_ scheme: ColorScheme, defaults: UserDefaults = AppGroup.defaults) -> WallpaperChoice {
        if let saved = defaults.string(forKey: key(scheme)).flatMap(WallpaperChoice.init(rawValue:)) { return saved }
        if let old = defaults.string(forKey: Wallpaper.legacyKey(scheme)).flatMap(Wallpaper.init(rawValue:)) {
            return WallpaperChoice(image: old)
        }
        return defaultFor(scheme)
    }

    /// Moves the old key over, once, at launch: an image kept there becomes the choice with no effect and no added
    /// fade, so the page looks as it did. Nothing changes when the new key is already there.
    static func migrate(defaults: UserDefaults = AppGroup.defaults) {
        for scheme in [ColorScheme.dark, .light] where defaults.object(forKey: Wallpaper.legacyKey(scheme)) != nil {
            if defaults.string(forKey: key(scheme)) == nil {
                defaults.set(initial(scheme, defaults: defaults).rawValue, forKey: key(scheme))
            }
            defaults.removeObject(forKey: Wallpaper.legacyKey(scheme))
        }
    }
}

/// The user's own photo, one per theme, kept in the app's container as it was picked (upright, at most 2,000 px; no
/// effect and no fade), so its effect and fade can change without picking it again. A photo saved before that was
/// kept with its effect and fade baked in, under the old name: it is drawn from as it is until a new one is picked.
enum OwnWallpaper {
    static var directory: URL {
        (try? FileManager.default.url(for: .applicationSupportDirectory, in: .userDomainMask, appropriateFor: nil, create: true))
            ?? FileManager.default.temporaryDirectory
    }

    static func sourceURL(_ scheme: ColorScheme, in dir: URL = directory) -> URL {
        dir.appendingPathComponent(scheme == .dark ? "wallpaper-dark-photo.jpg" : "wallpaper-light-photo.jpg")
    }

    /// The baked photo of earlier versions.
    static func legacyURL(_ scheme: ColorScheme, in dir: URL = directory) -> URL {
        dir.appendingPathComponent(scheme == .dark ? "wallpaper-dark.jpg" : "wallpaper-light.jpg")
    }

    /// The file to draw from: the kept photo, else an older baked one, else none.
    static func url(_ scheme: ColorScheme, in dir: URL = directory) -> URL? {
        let files = FileManager.default
        let source = sourceURL(scheme, in: dir)
        if files.fileExists(atPath: source.path) { return source }
        let legacy = legacyURL(scheme, in: dir)
        return files.fileExists(atPath: legacy.path) ? legacy : nil
    }

    static func exists(_ scheme: ColorScheme) -> Bool { url(scheme) != nil }

    static func load(_ scheme: ColorScheme, in dir: URL = directory) -> UIImage? {
        url(scheme, in: dir).flatMap { UIImage(contentsOfFile: $0.path) }
    }

    /// Keeps a newly picked photo (already upright and sized), replacing the theme's earlier one, baked or not.
    static func save(_ photo: CGImage, _ scheme: ColorScheme, in dir: URL = directory) throws {
        guard let data = UIImage(cgImage: photo).jpegData(compressionQuality: 0.92) else { throw CocoaError(.fileWriteUnknown) }
        try data.write(to: sourceURL(scheme, in: dir), options: .atomic)
        try? FileManager.default.removeItem(at: legacyURL(scheme, in: dir))
    }

    /// Bumped on each new photo (`WallpaperArt.photo`), so a photo picked under the same choice is drawn anew.
    static func revisionKey(_ scheme: ColorScheme) -> String { scheme == .dark ? "wallpaper2.dark.photo" : "wallpaper2.light.photo" }
}

/// The wallpaper layer at the top of a page, fixed behind everything (it never scrolls; the page and its panes scroll
/// over it, as on Android): the theme's art (`WallpaperArt`: the image with its effect and fade) across the full width
/// (its 780 × 1000 art is 390 × 500 at the boards' size), already faded into the page at its bottom, and a shade under
/// the status bar so the title reads. While a new choice renders, the last art stays.
struct WallpaperLayer: View {
    @Environment(\.colorScheme) private var scheme
    @AppStorage(WallpaperChoice.key(.dark), store: AppGroup.defaults) private var dark = WallpaperChoice.initial(.dark)
    @AppStorage(WallpaperChoice.key(.light), store: AppGroup.defaults) private var light = WallpaperChoice.initial(.light)
    @AppStorage(OwnWallpaper.revisionKey(.dark), store: AppGroup.defaults) private var darkPhoto = 0
    @AppStorage(OwnWallpaper.revisionKey(.light), store: AppGroup.defaults) private var lightPhoto = 0
    @State private var drawn: WallpaperArt.Drawn?

    var body: some View {
        let isDark = scheme == .dark
        let art = WallpaperArt.shown(isDark ? dark : light, photo: isDark ? darkPhoto : lightPhoto, scheme: scheme, hasPhoto: OwnWallpaper.exists(scheme))
        let shown = art.ready() ?? drawn.flatMap { $0.art.dark == isDark ? $0 : nil }
        GeometryReader { geo in
            ZStack(alignment: .top) {
                let shade = isDark ? Color.black.opacity(0.55) : Color(red: 246 / 255, green: 246 / 255, blue: 248 / 255).opacity(0.75)
                if let shown {
                    Image(uiImage: shown.image)
                        .resizable()
                        .interpolation(.medium)
                        .scaledToFill()
                        .frame(width: geo.size.width, height: geo.size.width * 500 / 390, alignment: .top)
                        .clipped()
                        // The frosted copy follows the art drawn: its key names the image, effect, fade and theme.
                        .backdropArt(key: shown.art.key, image: shown.image, dark: isDark, shade: shade)
                }
                LinearGradient(colors: [shade, .clear], startPoint: .top, endPoint: .bottom)
                    .frame(height: 150)
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .top)
        }
        .ignoresSafeArea()
        .allowsHitTesting(false)
        .accessibilityHidden(true)
        .task(id: art) {
            if let made = await art.drawn() { drawn = made }
        }
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
    /// glass, tracking where it is on screen as it moves and which art it is (`key`). `backdrop`: the shared one when
    /// nil (tests pass their own).
    func backdropArt(key: String, image: UIImage, dark: Bool, shade: Color = .clear, backdrop: Backdrop? = nil) -> some View {
        modifier(BackdropArt(key: key, image: image, dark: dark, shade: shade, backdrop: backdrop))
    }
}

/// Registers the art on both kinds of change: where it is (a new frame) and what it is (a new key: another image,
/// effect, fade, photo or theme, or a render that just finished). A new art drawn in the same place has the same frame,
/// so the frame alone kept the first art's frosted copy for good.
private struct BackdropArt: ViewModifier {
    let key: String
    let image: UIImage
    let dark: Bool
    let shade: Color
    let backdrop: Backdrop?
    @State private var box: CGRect?

    func body(content: Content) -> some View {
        content
            .onGeometryChange(for: CGRect.self) { $0.frame(in: .global) } action: { box in
                self.box = box
                register(in: box)
            }
            .onChange(of: key) {
                if let box { register(in: box) }
            }
    }

    private func register(in box: CGRect) {
        (backdrop ?? .shared).register(key: key, art: image, dark: dark, frame: Backdrop.fillRect(image: image.size, in: box), shade: shade)
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
