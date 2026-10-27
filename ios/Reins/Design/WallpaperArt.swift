import SwiftUI
import UIKit

/// A theme's wallpaper as it is drawn: the chosen image with its effect and fade. A shipped image with no effect and no
/// added fade is its asset, as it ships; anything else is rendered off the main actor (`WallpaperRender`) from the
/// asset or your kept photo, and kept in Caches under a name made of everything that changes it (`key`), so each one
/// is drawn once.
struct WallpaperArt: Hashable {
    var choice: WallpaperChoice
    var dark: Bool
    /// Your photo's revision (`OwnWallpaper.revisionKey`; 0 for a shipped image), so a new photo under the same choice
    /// is drawn anew.
    var photo: Int

    init(choice: WallpaperChoice, dark: Bool, photo: Int = 0) {
        self.choice = choice
        self.dark = dark
        self.photo = choice.image == .own ? photo : 0
    }

    /// Art and the image drawn for it.
    struct Drawn {
        var art: WallpaperArt
        var image: UIImage
    }

    /// What a theme shows: its choice, or the theme's default image (with the chosen effect and fade) when your photo
    /// is chosen but gone.
    static func shown(_ choice: WallpaperChoice, photo: Int, scheme: ColorScheme, hasPhoto: Bool) -> WallpaperArt {
        var choice = choice
        if choice.image == .own, !hasPhoto { choice.image = .defaultFor(scheme) }
        return WallpaperArt(choice: choice, dark: scheme == .dark, photo: photo)
    }

    /// The cache's name for the art, and the frosted copy's: theme, image, effect, fade in hundredths, and for your
    /// photo its revision.
    var key: String {
        var parts = [dark ? "dark" : "light", choice.image.rawValue, choice.effect.rawValue, "f\(choice.hundredths)"]
        if choice.image == .own { parts.append("p\(photo)") }
        return parts.joined(separator: "_")
    }

    /// A shipped image as it ships: drawn from its asset, nothing rendered.
    var isAsset: Bool { choice.image != .own && choice.effect == .original && choice.hundredths == 0 }

    // MARK: Drawing

    /// The last few drawn in this run, so going back and forth between choices is instant.
    @MainActor private static var memory: [String: UIImage] = [:]
    @MainActor private static var recent: [String] = []
    /// Renders under way, so two views asking for the same art share one.
    @MainActor private static var rendering: [String: Task<CGImage?, Never>] = [:]

    /// Ready now, without waiting: the asset, art drawn earlier in this run, or art drawn before and kept on disk (so
    /// the page opens with it, as it did when your photo was one baked file).
    @MainActor func ready() -> Drawn? {
        if isAsset { return UIImage(named: choice.image.assetName).map { Drawn(art: self, image: $0) } }
        if let image = Self.memory[key] { return Drawn(art: self, image: image) }
        guard let cached = UIImage(contentsOfFile: Self.cacheDirectory.appendingPathComponent(key + ".jpg").path)?.cgImage else { return nil }
        let image = UIImage(cgImage: cached, scale: 2, orientation: .up)
        Self.remember(key, image)
        return Drawn(art: self, image: image)
    }

    /// The art: ready, or rendered off the main actor; nil when there is nothing to draw from or the caller moved on. A
    /// short pause first lets a dragged slider settle.
    @MainActor func drawn() async -> Drawn? {
        if let ready = ready() { return ready }
        try? await Task.sleep(for: .milliseconds(60))
        guard !Task.isCancelled else { return nil }
        let key = key
        let task: Task<CGImage?, Never>
        if let running = Self.rendering[key] {
            task = running
        } else {
            let art = self
            task = Task.detached(priority: .userInitiated) { art.make() }
            Self.rendering[key] = task
        }
        let made = await task.value
        Self.rendering[key] = nil
        guard let made else { return nil }
        let image = UIImage(cgImage: made, scale: 2, orientation: .up)
        Self.remember(key, image)
        return Task.isCancelled ? nil : Drawn(art: self, image: image)
    }

    @MainActor private static func remember(_ key: String, _ image: UIImage) {
        memory[key] = image
        recent.removeAll { $0 == key }
        recent.append(key)
        while recent.count > 4 { memory[recent.removeFirst()] = nil }
    }

    /// Off the main actor: the cached file, or a render from the image, saved to the cache for next time.
    func make(cache: URL = WallpaperArt.cacheDirectory) -> CGImage? {
        let file = cache.appendingPathComponent(key + ".jpg")
        if let cached = UIImage(contentsOfFile: file.path)?.preparingForDisplay()?.cgImage { return cached }
        guard let source = Self.source(choice.image, dark: dark),
              let art = WallpaperRender.render(source, effect: choice.effect, fade: choice.fade, dark: dark) else { return nil }
        if let data = UIImage(cgImage: art).jpegData(compressionQuality: 0.9) {
            try? FileManager.default.createDirectory(at: cache, withIntermediateDirectories: true)
            if (try? data.write(to: file, options: .atomic)) != nil { Self.prune(cache, dark: dark, keeping: file) }
        }
        return art
    }

    static var cacheDirectory: URL {
        let caches = FileManager.default.urls(for: .cachesDirectory, in: .userDomainMask).first ?? FileManager.default.temporaryDirectory
        return caches.appendingPathComponent("wallpaper", isDirectory: true)
    }

    /// The image an effect is drawn on: the shipped asset, or your photo as it was picked (an older baked one as it is).
    static func source(_ image: Wallpaper, dark: Bool) -> CGImage? {
        image == .own ? OwnWallpaper.load(dark ? .dark : .light)?.cgImage : UIImage(named: image.assetName)?.cgImage
    }

    /// Keeps the theme's newest `count` files in the cache (`keeping` among them), so a dragged slider leaves no trail.
    static func prune(_ cache: URL, dark: Bool, keeping file: URL, count: Int = 3) {
        let files = FileManager.default
        let prefix = (dark ? "dark" : "light") + "_"
        guard let all = try? files.contentsOfDirectory(at: cache, includingPropertiesForKeys: [.contentModificationDateKey]) else { return }
        let others = all
            .filter { $0.lastPathComponent.hasPrefix(prefix) && $0.lastPathComponent != file.lastPathComponent }
            .map { ($0, (try? $0.resourceValues(forKeys: [.contentModificationDateKey]).contentModificationDate) ?? .distantPast) }
            .sorted { $0.1 > $1.1 }
        for (url, _) in others.dropFirst(max(0, count - 1)) { try? files.removeItem(at: url) }
    }

    /// The image with each effect, small and with no added fade: the Effect row's thumbnails. Off the main actor.
    static func thumbs(_ image: Wallpaper, dark: Bool) -> [WallpaperEffect: CGImage] {
        guard let source = source(image, dark: dark) else { return [:] }
        var out: [WallpaperEffect: CGImage] = [:]
        for effect in WallpaperEffect.allCases {
            out[effect] = WallpaperRender.render(source, effect: effect, fade: 0, dark: dark, size: CGSize(width: 156, height: 200))
        }
        return out
    }
}
