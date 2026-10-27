import CoreImage
import SwiftUI
import UIKit

/// The art behind Activity (the Wallpaper and Settings boards): images faded into the page at the bottom, one for dark
/// and one for light (Halo with Mono, and Torii with Mono, by default), or the user's own photo. Which image is one of
/// a theme's three settings (`WallpaperChoice`); the effect and the fade are the others. Groups over it become frosted
/// panes; Reduce Transparency makes the panes solid and keeps the art.
///
/// Every image takes every effect. Its `defaultEffect` is the boards' look for it. Eyes, Hands and Torii ship as they
/// were before any effect. Halo, Crescent, Figure and Ruins ship with their default effect already in them (`treated`):
/// that file is drawn for the default effect, and every other effect (and Original) is drawn from the image's
/// untreated original (`originalAssetName`, the boards' crop of the owner's art; `scripts/brand/wallpaper-originals.py`).
/// Android has the same images, defaults and rules (`Wallpaper.kt`).
enum Wallpaper: String, CaseIterable, Identifiable {
    case halo, crescent, figure, hands, ruins, eyes, torii
    /// The user's own photo (`OwnWallpaper`).
    case own

    var id: String { rawValue }

    static let shipped: [Wallpaper] = [.halo, .crescent, .figure, .hands, .ruins, .eyes, .torii]

    var title: String {
        switch self {
        case .halo: "Halo"
        case .crescent: "Crescent"
        case .figure: "Figure"
        case .hands: "Hands"
        case .ruins: "Ruins"
        case .eyes: "Eyes"
        case .torii: "Torii"
        case .own: "Your photo"
        }
    }

    /// The shipped file.
    var assetName: String { "wallpaper-\(rawValue)" }

    /// The file effects are drawn on: the untreated original of a treated image, else the shipped file.
    var originalAssetName: String { treated ? "wallpaper-\(rawValue)-original" : assetName }

    /// The effect picking the image gives it: the boards' look for it.
    var defaultEffect: WallpaperEffect {
        switch self {
        case .halo, .torii: .mono
        case .crescent, .eyes: .scanlines
        case .figure: .halftone
        case .ruins: .dither
        case .hands, .own: .original
        }
    }

    /// Whether the shipped file has `defaultEffect` already in it (and an untreated original ships beside it).
    var treated: Bool {
        switch self {
        case .halo, .crescent, .figure, .ruins: true
        case .hands, .eyes, .torii, .own: false
        }
    }

    /// Where the image already fades into the page (the boards baked it in): an effect fades out over it.
    var foot: WallpaperEffects.Foot? {
        switch self {
        case .halo: .init(start: 0.45, end: 0.97)
        case .crescent, .figure, .ruins, .eyes: .init(start: 0.5, end: 0.97)
        case .hands: .init(start: 0.62, end: 0.98)
        case .torii: .init(start: 0.55, end: 0.97)
        case .own: nil
        }
    }

    /// The effect that draws the image as it shipped before images came as originals: Eyes shipped with its scanlines
    /// in it; Torii shipped as the photo (its dither was the separate Torii dither); the treated images as they ship.
    var shippedLook: WallpaperEffect { self == .torii ? .original : defaultEffect }

    /// Where the image alone was kept before the effect and the fade were settings of their own; read once, by
    /// `WallpaperChoice.migrate`.
    static func legacyKey(_ scheme: ColorScheme) -> String { scheme == .dark ? "wallpaper.dark" : "wallpaper.light" }

    static func defaultFor(_ scheme: ColorScheme) -> Wallpaper { scheme == .dark ? .halo : .torii }

    /// The image itself, before any effect (your photo as it was picked), or nil when your photo is chosen but missing.
    func image(_ scheme: ColorScheme) -> UIImage? {
        self == .own ? OwnWallpaper.load(scheme) : UIImage(named: originalAssetName)
    }
}

/// A theme's wallpaper as three separate settings: the image (a shipped one or your photo), the effect over it and how
/// far it fades into the page. Kept as `image|effect|fade` (the fade in hundredths) under `key(scheme)` in the app
/// group, where `WallpaperLayer` reads it.
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

    /// Whether this is drawn from the shipped file of an image that ships with `effect` already in it.
    var fromShipped: Bool { image.treated && effect == image.defaultEffect }

    /// The effect drawn on the picture (`WallpaperArt.source`): none when the shipped file has it already.
    var applied: WallpaperEffect { fromShipped ? .original : effect }

    /// The picture this is drawn on, as it is: what shows while it renders.
    var base: WallpaperChoice { WallpaperChoice(image: image, effect: fromShipped ? effect : .original) }

    /// `image` as picking it gives it: with its own effect, no fade.
    static func of(_ image: Wallpaper) -> WallpaperChoice { WallpaperChoice(image: image, effect: image.defaultEffect) }

    /// `image` picked in this choice's place: with its own effect (`Wallpaper.defaultEffect`); the fade stays.
    func with(image: Wallpaper) -> WallpaperChoice { WallpaperChoice(image: image, effect: image.defaultEffect, fade: fade) }

    static func step(_ fade: Double) -> Double { (min(1, max(0, fade)) * 100).rounded() / 100 }

    /// Where a theme's choice is kept: the fourth key, since every image takes every effect.
    static func key(_ scheme: ColorScheme) -> String { scheme == .dark ? "wallpaper4.dark" : "wallpaper4.light" }

    /// Where it was kept before every image took every effect; read once (`treatedAsShipped`).
    static func thirdKey(_ scheme: ColorScheme) -> String { scheme == .dark ? "wallpaper3.dark" : "wallpaper3.light" }

    /// Where it was kept before the shipped images came as originals; read once (`fromBefore`).
    static func previousKey(_ scheme: ColorScheme) -> String { scheme == .dark ? "wallpaper2.dark" : "wallpaper2.light" }

    /// The theme's shipped default: Halo as it ships (with Mono) in the dark, Torii with Mono in the light.
    static func defaultFor(_ scheme: ColorScheme) -> WallpaperChoice { of(Wallpaper.defaultFor(scheme)) }

    /// A choice kept before every image took every effect: Halo, Crescent, Figure and Ruins had no effect of their own
    /// and were drawn as they ship, which is now their own effect. Others are kept as they are.
    static func treatedAsShipped(_ choice: WallpaperChoice) -> WallpaperChoice {
        choice.image.treated ? WallpaperChoice(image: choice.image, effect: choice.image.defaultEffect, fade: choice.fade) : choice
    }

    /// A choice kept before the shipped images came as originals (`image|effect|hundredths`, or the old key's bare
    /// image), drawn as it was: Original on a shipped image meant the picture as shipped (`Wallpaper.shippedLook`:
    /// Eyes with Scanlines), and so did the old Colour (a lift of the picture's colour). Torii dither is Torii with
    /// Mono. An image that ships treated was drawn as it ships whatever its effect: it keeps that look
    /// (`treatedAsShipped`). Nil for an image no longer known.
    static func fromBefore(_ raw: String) -> WallpaperChoice? {
        let parts = raw.split(separator: "|", omittingEmptySubsequences: false).map(String.init)
        guard parts.count == 1 || parts.count == 3 else { return nil }
        let toriiDither = parts[0] == "torii-dither"
        guard let image = toriiDither ? .torii : Wallpaper(rawValue: parts[0]) else { return nil }
        let old = parts.count == 3 ? parts[1] : "original"
        let fade = parts.count == 3 ? Double(Int(parts[2]) ?? 0) / 100 : 0
        let asShipped = old == "original" || old == "colour"
        let effect: WallpaperEffect = if toriiDither && asShipped {
            .mono
        } else if asShipped {
            image.shippedLook
        } else {
            WallpaperEffect(rawValue: old) ?? image.shippedLook
        }
        return treatedAsShipped(WallpaperChoice(image: image, effect: effect, fade: fade))
    }

    /// What a theme draws: the saved choice; before one was saved under this key, the one kept before every image took
    /// every effect (`treatedAsShipped`), before the originals, or the image the oldest key kept, looking as it did
    /// (`fromBefore`), or the default.
    static func initial(_ scheme: ColorScheme, defaults: UserDefaults = AppGroup.defaults) -> WallpaperChoice {
        if let saved = defaults.string(forKey: key(scheme)).flatMap(WallpaperChoice.init(rawValue:)) { return saved }
        if let third = defaults.string(forKey: thirdKey(scheme)).flatMap(WallpaperChoice.init(rawValue:)) { return treatedAsShipped(third) }
        if let kept = defaults.string(forKey: previousKey(scheme)).flatMap(fromBefore) { return kept }
        if let old = defaults.string(forKey: Wallpaper.legacyKey(scheme)).flatMap(fromBefore) { return old }
        return defaultFor(scheme)
    }

    /// Moves the earlier keys over, once, at launch: a choice kept there becomes this key's, looking as it did. Nothing
    /// changes when this key is already there. Art drawn with the old effects (and from the old Eyes) is dropped.
    static func migrate(defaults: UserDefaults = AppGroup.defaults, caches: URL? = nil) {
        var moved = false
        for scheme in [ColorScheme.dark, .light] {
            let earlier = [thirdKey(scheme), previousKey(scheme), Wallpaper.legacyKey(scheme)].filter { defaults.object(forKey: $0) != nil }
            guard !earlier.isEmpty else { continue }
            if defaults.string(forKey: key(scheme)) == nil {
                defaults.set(initial(scheme, defaults: defaults).rawValue, forKey: key(scheme))
            }
            earlier.forEach(defaults.removeObject(forKey:))
            // The third key's art is in the current cache, under keys that now mean the same picture or none.
            if earlier != [thirdKey(scheme)] { moved = true }
        }
        if moved { try? FileManager.default.removeItem(at: caches ?? WallpaperArt.previousCacheDirectory) }
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
/// the status bar so the title reads. While a new choice renders, the last art stays (with none, the picture it is drawn
/// on shows).
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
        // While new art renders: the last art for the theme, else the picture it is drawn on, as Android's.
        let shown = art.ready() ?? drawn.flatMap { $0.art.dark == isDark ? $0 : nil } ?? art.base.ready()
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

    /// Works on sRGB-encoded values, as Android's bitmaps do, so the same matrix and blur give the same colours.
    private static let sRGB = CGColorSpace(name: CGColorSpace.sRGB)!
    private static let context = CIContext(options: [.useSoftwareRenderer: false, .workingColorSpace: sRGB])

    /// The blur's sigma as a share of the art's width: Android's (`Wallpapers.frosted`), so shapes behind the glass
    /// read alike on both. Android, on the 780-px-wide art:
    /// - scales it to 1/12 (65 px wide) with bilinear sampling, which adds next to no blur;
    /// - box-blurs it twice, radius 2: each pass is one 5-tap box per axis, variance (5² − 1) / 12 = 2 px², so 4 px²
    ///   in all;
    /// - draws it back bilinearly, a 1-px tent, + 1/6 px².
    /// So σ = √(4 + 1/6) ≈ 2.04 px of the 1/12 copy, ≈ 24.5 px of the art, 3.14 % of its width: 12.3 pt on a 390-pt
    /// iPhone (12.9 dp on a 411-dp Android). It was σ 20 on the quarter copy here (80 px of the art, 40 pt), 3.3 times
    /// as much, which left a blob of light with no shape. Now 0.0314 × 195 ≈ 6.1 px on the quarter copy.
    nonisolated static let blurShare = 0.0314

    /// The bottom share of the copy that fades into the page's paper (eased), ending exactly on paper, so the glass has
    /// no edge where the art ends. The art itself fades over about its lower 40 %; Android fades the same share.
    nonisolated static let fadeShare = 0.3

    /// A quarter-size copy, saturated 1.9 and darkened to 0.82 (dark) or lifted to 1.06 (light) as Android's is (its
    /// `ColorMatrix.setSaturation` weights, before the blur and clamped), blurred by `blurShare` of its width, and faded
    /// into the paper over its bottom `fadeShare`. The edges are extended before the blur, so the copy never darkens at
    /// its sides.
    static func blur(_ art: UIImage, dark: Bool) -> UIImage? {
        guard let cg = art.cgImage else { return nil }
        guard let quarter = Pixels(fill: cg, width: max(4, cg.width / 4), height: max(4, cg.height / 4))?.image() else { return nil }
        let small = CIImage(cgImage: quarter)
        let extent = small.extent
        let s = 1.9, k = dark ? 0.82 : 1.06
        let r = 0.213 * (1 - s), g = 0.715 * (1 - s), b = 0.072 * (1 - s)
        let tuned = small.applyingFilter("CIColorMatrix", parameters: [
            "inputRVector": CIVector(x: k * (r + s), y: k * g, z: k * b, w: 0),
            "inputGVector": CIVector(x: k * r, y: k * (g + s), z: k * b, w: 0),
            "inputBVector": CIVector(x: k * r, y: k * g, z: k * (b + s), w: 0),
            "inputAVector": CIVector(x: 0, y: 0, z: 0, w: 1),
            "inputBiasVector": CIVector(x: 0, y: 0, z: 0, w: 0),
        ]).applyingFilter("CIColorClamp")
        let blurred = tuned.clampedToExtent().applyingGaussianBlur(sigma: blurShare * Double(extent.width)).cropped(to: extent)
        guard let out = context.createCGImage(blurred, from: extent, format: .RGBA8, colorSpace: sRGB),
              var px = Pixels(fill: out, width: out.width, height: out.height) else { return nil }
        fadeIntoPaper(&px, dark: dark)
        return px.image().map { UIImage(cgImage: $0) }
    }

    /// Blends the copy's bottom `fadeShare` into the page's paper, eased, its last row exactly paper.
    nonisolated static func fadeIntoPaper(_ px: inout Pixels, dark: Bool) {
        let paper = WallpaperRender.paper(dark: dark)
        let target = [Double(paper.r), Double(paper.g), Double(paper.b)]
        let start = Double(px.height) * (1 - fadeShare)
        let last = Double(px.height - 1)
        for y in max(0, Int(start.rounded(.down)))..<px.height {
            let t = min(1, max(0, (Double(y) - start) / max(1, last - start)))
            let e = t * t * (3 - 2 * t)
            for x in 0..<px.width {
                let i = px.index(x, y)
                for c in 0..<3 {
                    px.bytes[i + c] = UInt8(min(255, max(0, (Double(px.bytes[i + c]) * (1 - e) + target[c] * e).rounded())))
                }
            }
        }
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
