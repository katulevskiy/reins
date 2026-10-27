import PhotosUI
import SwiftUI
import UIKit

/// Settings > Wallpaper (the Settings board, 04b and 04c), quiet: for dark and for light, three separate settings
/// (`WallpaperChoice`): the image (the shipped ones or your photo), the effect over it and the fade. Picking an image
/// gives it its own effect (`Wallpaper.defaultEffect`), which can then change; an image that ships treated has no
/// Effect row. A small Activity shows the result; every change applies at once. The choice is kept under `WallpaperChoice.key(scheme)` in
/// the app group, where `WallpaperLayer` reads it; your photo is kept as picked (`OwnWallpaper`), so its effect and fade
/// can change later.
struct WallpaperScreen: View {
    @Environment(\.colorScheme) private var systemScheme
    @Environment(\.feedback) private var feedback
    @AppStorage(WallpaperChoice.key(.dark), store: AppGroup.defaults) private var dark = WallpaperChoice.initial(.dark)
    @AppStorage(WallpaperChoice.key(.light), store: AppGroup.defaults) private var light = WallpaperChoice.initial(.light)
    @AppStorage(OwnWallpaper.revisionKey(.dark), store: AppGroup.defaults) private var darkPhoto = 0
    @AppStorage(OwnWallpaper.revisionKey(.light), store: AppGroup.defaults) private var lightPhoto = 0
    /// Which theme's wallpaper is being set: Dark first, as the board opens.
    @State private var picking: ColorScheme? = .dark
    @State private var item: PhotosPickerItem?
    @State private var importing = false
    @State private var error: String?
    /// The art last drawn for the preview, kept while the next renders.
    @State private var preview: WallpaperArt.Drawn?
    /// The current image with each effect, and which image (theme, photo) they were drawn from.
    @State private var thumbs: (source: ThumbSource, images: [WallpaperEffect: CGImage])?
    /// Your photo, small, for its tile.
    @State private var photoThumb: (source: ThumbSource, image: UIImage)?
    /// The shipped images as picking them gives them, and for which theme.
    @State private var imageThumbs: (dark: Bool, images: [Wallpaper: UIImage])?

    private var scheme: ColorScheme { picking ?? systemScheme }
    private var choice: WallpaperChoice { scheme == .dark ? dark : light }
    private var photo: Int { scheme == .dark ? darkPhoto : lightPhoto }

    private struct ThumbSource: Hashable {
        var image: Wallpaper
        var dark: Bool
        var photo: Int
    }

    var body: some View {
        let hasPhoto = OwnWallpaper.exists(scheme)
        let art = WallpaperArt.shown(choice, photo: photo, scheme: scheme, hasPhoto: hasPhoto)
        let source = ThumbSource(image: art.choice.image, dark: art.dark, photo: art.photo)
        let ownSource = ThumbSource(image: .own, dark: art.dark, photo: photo)
        QuietPage("Wallpaper") {
            SchemeSegments(scheme: Binding(get: { scheme }, set: { picking = $0 }))
                .padding(.horizontal, 20)
                .padding(.top, 4)
            PhonePreview(image: (art.ready() ?? preview.flatMap { $0.art.dark == art.dark ? $0 : nil })?.image, dark: art.dark)
                .frame(maxWidth: .infinity)
                .padding(.top, 22)
                .padding(.bottom, 8)
            SectionHeader("Image", quiet: true)
            ScrollView(.horizontal, showsIndicators: false) {
                HStack(alignment: .top, spacing: 10) {
                    ForEach(Wallpaper.shipped) { wallpaper in
                        let thumb = imageThumbs.flatMap { $0.dark == art.dark ? $0.images[wallpaper] : nil }
                            ?? (wallpaper.defaultEffect == .original ? wallpaper.image(scheme) : nil)
                        ImageChoice(wallpaper: wallpaper, image: thumb, selected: art.choice.image == wallpaper) {
                            pick(wallpaper)
                        }
                    }
                    if hasPhoto {
                        let thumb = photoThumb.flatMap { $0.source == ownSource ? $0.image : nil }
                        ImageChoice(wallpaper: .own, image: thumb, selected: art.choice.image == .own) { pick(.own) }
                    }
                    PhotosPicker(selection: $item, matching: .images, photoLibrary: .shared()) {
                        OwnPhotoTile(replacing: hasPhoto, busy: importing)
                    }
                    .buttonStyle(PressDimsStyle())
                    .disabled(importing)
                    .accessibilityLabel(hasPhoto ? "Replace your photo" : "Your photo")
                    .accessibilityIdentifier("ownWallpaper")
                }
                .padding(.horizontal, 20)
                .padding(.vertical, 4)
            }
            if art.choice.image.takesEffects {
                SectionHeader("Effect", quiet: true)
                HStack(alignment: .top, spacing: 0) {
                    ForEach(WallpaperEffect.allCases) { effect in
                        EffectChoice(effect: effect, thumb: thumbs.flatMap { $0.source == source ? $0.images[effect] : nil }, selected: art.choice.effect == effect) {
                            guard choice.effect != effect else { return }
                            feedback.play(.selection)
                            update { $0.effect = effect }
                        }
                        if effect != WallpaperEffect.allCases.last { Spacer(minLength: 4) }
                    }
                }
                .padding(.horizontal, 16)
                .padding(.top, 4)
            }
            HStack(spacing: 12) {
                GlyphIcon(.appearance, size: 16).foregroundStyle(Palette.inkMeta).accessibilityHidden(true)
                Slider(value: Binding(get: { choice.fade }, set: { value in update { $0.fade = value } }), in: 0...1) { editing in
                    if !editing { feedback.play(.selection) }
                }
                .tint(Palette.violet)
                .accessibilityLabel("Fade")
                .accessibilityIdentifier("wallpaperFade")
                Text("Fade").font(RFont.sans(13)).foregroundStyle(Palette.ink2).accessibilityHidden(true)
            }
            .padding(.horizontal, 24)
            .padding(.top, 22)
            if let error { QuietBanner(text: error) }
        }
        .onChange(of: item) { _, picked in
            guard let picked else { return }
            item = nil
            let scheme = scheme
            Task { await importPhoto(picked, for: scheme) }
        }
        .task(id: art) {
            if let drawn = await art.drawn() { preview = drawn }
        }
        .task(id: art.dark) {
            let dark = art.dark
            let images = await Task.detached(priority: .utility) { WallpaperArt.imageThumbs(dark: dark) }.value
            if !Task.isCancelled { imageThumbs = (dark, images.mapValues { UIImage(cgImage: $0) }) }
        }
        .task(id: source) {
            let images = await Task.detached(priority: .utility) { WallpaperArt.thumbs(source.image, dark: source.dark) }.value
            if !Task.isCancelled { thumbs = (source, images) }
        }
        .task(id: hasPhoto ? ownSource : nil) {
            guard hasPhoto else { return }
            let dark = ownSource.dark
            let image = await Task.detached(priority: .utility) {
                OwnWallpaper.load(dark ? .dark : .light)?.preparingThumbnail(of: CGSize(width: 208, height: 270))
            }.value
            if !Task.isCancelled, let image { photoThumb = (ownSource, image) }
        }
    }

    private func update(_ change: (inout WallpaperChoice) -> Void) {
        if scheme == .dark { change(&dark) } else { change(&light) }
    }

    private func pick(_ wallpaper: Wallpaper) {
        guard choice.image != wallpaper else { return }
        feedback.play(.selection)
        update { $0 = $0.with(image: wallpaper) }
    }

    /// Keeps a picked photo for the theme as it is (upright, at most 2,000 px) and chooses it, keeping the effect.
    /// A photo needs a fade into the page: with none set, it opens with the default.
    private func importPhoto(_ picked: PhotosPickerItem, for scheme: ColorScheme) async {
        error = nil
        importing = true
        defer { importing = false }
        do {
            guard let data = try await picked.loadTransferable(type: Data.self) else {
                error = "That photo could not be opened."
                return
            }
            let saved = await Task.detached(priority: .userInitiated) { () -> Bool in
                guard let image = UIImage(data: data), let upright = WallpaperRender.upright(image, maxSide: 2_000) else { return false }
                return (try? OwnWallpaper.save(upright, scheme)) != nil
            }.value
            guard saved else {
                feedback.play(.error)
                error = "That photo could not be opened."
                return
            }
            let revision = Int(Date().timeIntervalSince1970 * 1_000)
            if scheme == .dark { darkPhoto = revision } else { lightPhoto = revision }
            var next = scheme == .dark ? dark : light
            next.image = .own
            if next.hundredths == 0 { next.fade = WallpaperRender.defaultFade }
            if scheme == .dark { dark = next } else { light = next }
            feedback.play(.selection)
        } catch {
            feedback.play(.error)
            self.error = error.userMessage
        }
    }
}

/// Dark · Light, each with its glyph, in the kit's sunken well (the board's `cho`).
struct SchemeSegments: View {
    @Binding var scheme: ColorScheme
    @Environment(\.feedback) private var feedback

    var body: some View {
        HStack(spacing: 2) {
            segment(.dark, "Dark", .moon)
            segment(.light, "Light", .appearance)
        }
        .padding(4)
        .background(Palette.paperSunken, in: Capsule())
        .accessibilityElement(children: .contain)
    }

    private func segment(_ value: ColorScheme, _ title: String, _ glyph: Glyph) -> some View {
        let on = scheme == value
        return Button {
            guard !on else { return }
            feedback.play(.selection)
            scheme = value
        } label: {
            HStack(spacing: 4) {
                GlyphIcon(glyph, size: 15)
                Text(title).font(RFont.sans(14, .semibold)).lineLimit(1)
            }
            .foregroundStyle(on ? Palette.ink : Palette.ink2)
            .frame(maxWidth: .infinity, minHeight: 38)
            .background {
                if on {
                    Capsule().fill(Palette.paperRaised)
                        .overlay(Capsule().strokeBorder(Palette.rule, lineWidth: 1))
                        .shadow(color: .black.opacity(0.08), radius: 0.5, y: 1)
                }
            }
            .contentShape(Capsule())
        }
        .buttonStyle(.plain)
        .accessibilityAddTraits(on ? [.isButton, .isSelected] : .isButton)
        .accessibilityIdentifier("wallpaperScheme:\(title.lowercased())")
    }
}

/// The art behind a small Activity, in a phone's frame: what the page will look like.
private struct PhonePreview: View {
    var image: UIImage?
    var dark: Bool

    var body: some View {
        let shape = RoundedRectangle(cornerRadius: 30, style: .continuous)
        let paper = dark ? Color(uiColor: UIColor(hex: 0x121215)) : Color(uiColor: UIColor(hex: 0xF6F6F8))
        let bar = dark ? Color(uiColor: UIColor(hex: 0x1C1C21)) : Color.white
        ZStack(alignment: .topLeading) {
            paper
            if let image {
                Image(uiImage: image).resizable().interpolation(.medium).frame(width: 220, height: 282)
            }
            Text("Activity")
                .font(RFont.fixedSans(20, .medium))
                .tracking(-0.4)
                .foregroundStyle(dark ? .white : Palette.onFieldInk)
                .padding(.leading, 14)
                .padding(.top, 26)
            VStack(spacing: 6) {
                ForEach(0..<4, id: \.self) { _ in
                    RoundedRectangle(cornerRadius: 9, style: .continuous).fill(bar).frame(height: 30)
                }
            }
            .padding(.horizontal, 10)
            .padding(.top, 190)
        }
        .frame(width: 220, height: 400)
        .clipShape(shape)
        .overlay(shape.strokeBorder(Color(uiColor: UIColor(hex: 0x111114)), lineWidth: 5).padding(-5))
        .shadow(color: .black.opacity(0.35), radius: 20, y: 20)
        .environment(\.colorScheme, dark ? .dark : .light)
        .accessibilityHidden(true)
    }
}

/// An image as a small portrait, its top showing (the board's `thumb`): ringed in violet with a check when chosen,
/// its name under it.
private struct ImageChoice: View {
    var wallpaper: Wallpaper
    var image: UIImage?
    var selected: Bool
    var action: () -> Void

    var body: some View {
        Button(action: action) {
            VStack(spacing: 6) {
                WallpaperThumb(image: image, width: 72, radius: 14)
                    .overlay {
                        if selected {
                            RoundedRectangle(cornerRadius: 16.5, style: .continuous).strokeBorder(Palette.violet, lineWidth: 2.5).padding(-2.5)
                        } else {
                            RoundedRectangle(cornerRadius: 14, style: .continuous).strokeBorder(Palette.rule, lineWidth: 1)
                        }
                    }
                    .overlay(alignment: .bottomTrailing) {
                        if selected {
                            GlyphIcon(.check, size: 12)
                                .foregroundStyle(.white)
                                .frame(width: 20, height: 20)
                                .background(Palette.violetFill, in: Circle())
                                .padding(5)
                        }
                    }
                Text(wallpaper.title)
                    .font(RFont.sans(11.5, selected ? .semibold : .regular))
                    .foregroundStyle(selected ? Palette.ink : Palette.ink2)
                    .lineLimit(1)
                    .minimumScaleFactor(0.8)
            }
            .frame(width: 72)
            .contentShape(Rectangle())
        }
        .buttonStyle(PressDimsStyle())
        .accessibilityLabel(wallpaper.title)
        .accessibilityAddTraits(selected ? [.isButton, .isSelected] : .isButton)
        .accessibilityIdentifier("wallpaper:\(wallpaper.rawValue)")
    }
}

/// The dashed tile that picks your photo, or a new one in its place.
private struct OwnPhotoTile: View {
    var replacing: Bool
    var busy: Bool

    var body: some View {
        VStack(spacing: 6) {
            RoundedRectangle(cornerRadius: 14, style: .continuous)
                .strokeBorder(Palette.rule2, style: StrokeStyle(lineWidth: 1.5, dash: [4, 3]))
                .frame(width: 72, height: 72 * 135 / 104)
                .overlay {
                    if busy { ProgressView() } else { GlyphIcon(.plus, size: 20) }
                }
            Text(replacing ? "Replace" : "Your photo")
                .font(RFont.sans(11.5))
                .lineLimit(1)
                .minimumScaleFactor(0.8)
        }
        .foregroundStyle(Palette.ink2)
        .frame(width: 72)
        .contentShape(Rectangle())
    }
}

/// An effect as a small square of the current image with it applied, its name under it; the chosen one is ringed in
/// violet.
private struct EffectChoice: View {
    var effect: WallpaperEffect
    var thumb: CGImage?
    var selected: Bool
    var action: () -> Void

    var body: some View {
        Button(action: action) {
            VStack(spacing: 6) {
                Color.clear
                    .frame(width: 52, height: 52)
                    .overlay {
                        if let thumb {
                            Image(decorative: thumb, scale: 1).resizable().scaledToFill()
                                // As the board's `object-position: 50% 20%`: the face, not the middle.
                                .offset(y: 8)
                        }
                    }
                    .background(Palette.paperSunken)
                    .clipShape(RoundedRectangle(cornerRadius: 14, style: .continuous))
                    .overlay {
                        if selected {
                            RoundedRectangle(cornerRadius: 16.5, style: .continuous).strokeBorder(Palette.violet, lineWidth: 2.5).padding(-2.5)
                        } else {
                            RoundedRectangle(cornerRadius: 14, style: .continuous).strokeBorder(Palette.rule, lineWidth: 1)
                        }
                    }
                Text(effect.title)
                    .font(RFont.sans(11.5, selected ? .semibold : .regular))
                    .foregroundStyle(selected ? Palette.ink : Palette.ink2)
                    .lineLimit(1)
                    .minimumScaleFactor(0.8)
            }
            .frame(width: 60)
            .contentShape(Rectangle())
        }
        .buttonStyle(PressDimsStyle())
        .accessibilityLabel(effect.title)
        .accessibilityAddTraits(selected ? [.isButton, .isSelected] : .isButton)
        .accessibilityIdentifier("effect:\(effect.rawValue)")
    }
}

/// An image cut to a rounded tile, its top showing: a portrait `width` wide (the Image row's 104 × 135), or a square
/// `side` (the Settings row's 32 pt).
struct WallpaperThumb: View {
    var image: UIImage?
    var width: CGFloat
    var height: CGFloat
    var radius: CGFloat

    init(image: UIImage?, width: CGFloat, radius: CGFloat) {
        self.init(image: image, width: width, height: width * 135 / 104, radius: radius)
    }

    init(image: UIImage?, side: CGFloat, radius: CGFloat) {
        self.init(image: image, width: side, height: side, radius: radius)
    }

    private init(image: UIImage?, width: CGFloat, height: CGFloat, radius: CGFloat) {
        self.image = image
        self.width = width
        self.height = height
        self.radius = radius
    }

    var body: some View {
        Color.clear
            .frame(width: width, height: height)
            .overlay(alignment: .top) {
                if let image {
                    Image(uiImage: image).resizable().interpolation(.medium).scaledToFill()
                }
            }
            .background(Palette.paperSunken)
            .clipShape(RoundedRectangle(cornerRadius: radius, style: .continuous))
            .accessibilityHidden(true)
    }
}

/// The theme's wallpaper as it is drawn (its effect and fade), as a small square: the Settings row's tile.
struct ChosenWallpaperThumb: View {
    var scheme: ColorScheme
    var side: CGFloat
    var radius: CGFloat
    @AppStorage(WallpaperChoice.key(.dark), store: AppGroup.defaults) private var dark = WallpaperChoice.initial(.dark)
    @AppStorage(WallpaperChoice.key(.light), store: AppGroup.defaults) private var light = WallpaperChoice.initial(.light)
    @AppStorage(OwnWallpaper.revisionKey(.dark), store: AppGroup.defaults) private var darkPhoto = 0
    @AppStorage(OwnWallpaper.revisionKey(.light), store: AppGroup.defaults) private var lightPhoto = 0
    @State private var drawn: WallpaperArt.Drawn?

    var body: some View {
        let isDark = scheme == .dark
        let art = WallpaperArt.shown(isDark ? dark : light, photo: isDark ? darkPhoto : lightPhoto, scheme: scheme, hasPhoto: OwnWallpaper.exists(scheme))
        WallpaperThumb(image: (art.ready() ?? drawn.flatMap { $0.art.dark == isDark ? $0 : nil })?.image, side: side, radius: radius)
            .task(id: art) {
                if let made = await art.drawn() { drawn = made }
            }
    }
}
