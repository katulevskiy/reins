import PhotosUI
import SwiftUI
import UIKit

/// Settings > Wallpaper (the Settings board, 04b and 04c), as Android's (`WallpaperScreen.kt`): one for dark and one
/// for light, each with three separate settings (`WallpaperChoice`): the image (the shipped ones or your photo), its
/// effect and how far it fades into the page. They apply as they are picked; picking an image gives it its own effect
/// (`Wallpaper.defaultEffect`), which can then change; every image has the Effect row. The fade follows the finger in
/// the preview and is kept when it lifts. The choice is kept under `WallpaperChoice.key(scheme)` in the app group, where
/// `WallpaperLayer` reads it; your photo is kept as picked (`OwnWallpaper`), so its effect and fade can change later.
struct WallpaperScreen: View {
    @Environment(\.colorScheme) private var systemScheme
    @Environment(\.feedback) private var feedback
    @AppStorage(WallpaperChoice.key(.dark), store: AppGroup.defaults) private var dark = WallpaperChoice.initial(.dark)
    @AppStorage(WallpaperChoice.key(.light), store: AppGroup.defaults) private var light = WallpaperChoice.initial(.light)
    @AppStorage(OwnWallpaper.revisionKey(.dark), store: AppGroup.defaults) private var darkPhoto = 0
    @AppStorage(OwnWallpaper.revisionKey(.light), store: AppGroup.defaults) private var lightPhoto = 0
    /// Which theme's wallpaper is being set; nil: the theme the app is in.
    @State private var picking: ColorScheme?
    @State private var item: PhotosPickerItem?
    @State private var choosingPhoto = false
    @State private var importing = false
    @State private var error: String?
    /// The fade while the slider is held: the preview follows it, and it is kept when the finger lifts.
    @State private var heldFade: Double?
    @State private var sliding = false
    /// The preview's picture and the art it shows, kept while the next is drawn.
    @State private var preview: (art: WallpaperArt, image: UIImage)?
    /// The current image with each effect, and which image (theme, photo) they were drawn from.
    @State private var thumbs: (source: ThumbSource, images: [WallpaperEffect: UIImage])?
    /// Every image as picking it gives it, for which theme and photo.
    @State private var imageThumbs: (source: ImageThumbSource, images: [Wallpaper: UIImage])?

    private var scheme: ColorScheme { picking ?? systemScheme }
    private var choice: WallpaperChoice { scheme == .dark ? dark : light }
    private var photo: Int { scheme == .dark ? darkPhoto : lightPhoto }
    /// The choice as the preview shows it: with the held fade while the slider is dragged.
    private var shownChoice: WallpaperChoice {
        var shown = choice
        if let heldFade { shown.fade = heldFade }
        return shown
    }

    private struct ThumbSource: Hashable {
        var image: Wallpaper
        var dark: Bool
        var photo: Int
    }

    private struct ImageThumbSource: Hashable {
        var dark: Bool
        var photo: Int
        var hasPhoto: Bool
    }

    var body: some View {
        let hasPhoto = OwnWallpaper.exists(scheme)
        let art = WallpaperArt.shown(shownChoice, photo: photo, scheme: scheme, hasPhoto: hasPhoto)
        let source = ThumbSource(image: art.choice.image, dark: art.dark, photo: art.photo)
        let imageSource = ImageThumbSource(dark: art.dark, photo: photo, hasPhoto: hasPhoto)
        QuietPage("Wallpaper") {
            Chooser([segment(.dark, "Dark"), segment(.light, "Light")])
                .padding(.horizontal, 20)
                .padding(.top, 6)
            PreviewCard(image: preview.flatMap { $0.art.dark == art.dark ? $0.image : nil } ?? art.base.ready()?.image, dark: art.dark)
                .frame(maxWidth: .infinity)
                .padding(.top, 14)

            WallpaperLabel("Image")
            imageRow(art: art, hasPhoto: hasPhoto, thumbs: imageThumbs.flatMap { $0.source == imageSource ? $0.images : nil } ?? [:])

            WallpaperLabel("Effect")
            effectRow(art: art, thumbs: thumbs.flatMap { $0.source == source ? $0.images : nil } ?? [:])

            WallpaperLabel("Fade")
            Slider(value: Binding(get: { heldFade ?? choice.fade }, set: setFade), in: 0...1) { editing in
                sliding = editing
                if !editing, let held = heldFade {
                    update { $0.fade = held }
                    heldFade = nil
                }
            }
            .tint(Palette.violetFill)
            .padding(.horizontal, 20)
            .accessibilityLabel("Fade")
            .accessibilityIdentifier("wallpaperFade")
            if let error { QuietBanner(text: error) }
        }
        .photosPicker(isPresented: $choosingPhoto, selection: $item, matching: .images, photoLibrary: .shared())
        .onChange(of: item) { _, picked in
            guard let picked else { return }
            item = nil
            let scheme = scheme
            Task { await importPhoto(picked, for: scheme) }
        }
        .onChange(of: scheme) { heldFade = nil }
        .task(id: art) {
            // A short pause lets a dragged slider move on before the last step is drawn.
            try? await Task.sleep(for: .milliseconds(16))
            guard !Task.isCancelled else { return }
            let made = await Task.detached(priority: .userInitiated) { WallpaperArt.preview(art) }.value
            if !Task.isCancelled, let made { preview = (art, UIImage(cgImage: made)) }
        }
        .task(id: imageSource) {
            let images = await Task.detached(priority: .utility) { WallpaperArt.imageThumbs(dark: imageSource.dark, photo: imageSource.photo) }.value
            if !Task.isCancelled { imageThumbs = (imageSource, images.mapValues { UIImage(cgImage: $0) }) }
        }
        .task(id: source) {
            let images = await Task.detached(priority: .utility) { WallpaperArt.thumbs(source.image, dark: source.dark, photo: source.photo) }.value
            if !Task.isCancelled { thumbs = (source, images.mapValues { UIImage(cgImage: $0) }) }
        }
    }

    private func segment(_ value: ColorScheme, _ title: String) -> Chooser.Segment {
        Chooser.Segment(id: "wallpaperScheme:\(title.lowercased())", title: title, selected: scheme == value) {
            feedback.play(.selection)
            picking = value
        }
    }

    // MARK: Image

    /// The shipped images, then your photo: 84-pt tiles 10 apart, opened with the chosen one second from the left.
    private func imageRow(art: WallpaperArt, hasPhoto: Bool, thumbs: [Wallpaper: UIImage]) -> some View {
        let images = Wallpaper.shipped + [.own]
        return ScrollViewReader { proxy in
            ScrollView(.horizontal, showsIndicators: false) {
                HStack(alignment: .top, spacing: 10) {
                    ForEach(images) { wallpaper in
                        imageTile(wallpaper, art: art, hasPhoto: hasPhoto, thumb: thumbs[wallpaper]).id(wallpaper)
                    }
                }
                .padding(.horizontal, 16)
            }
            .onAppear { scrollToChosen(proxy, images: images, chosen: art.choice.image) }
            .onChange(of: scheme) { scrollToChosen(proxy, images: images, chosen: (scheme == .dark ? dark : light).image) }
        }
    }

    private func scrollToChosen(_ proxy: ScrollViewProxy, images: [Wallpaper], chosen: Wallpaper) {
        guard let at = images.firstIndex(of: chosen) else { return }
        proxy.scrollTo(images[max(0, at - 1)], anchor: .leading)
    }

    @ViewBuilder
    private func imageTile(_ wallpaper: Wallpaper, art: WallpaperArt, hasPhoto: Bool, thumb: UIImage?) -> some View {
        let on = wallpaper == art.choice.image
        VStack(spacing: 6) {
            if wallpaper == .own && !hasPhoto {
                // No photo yet: a dashed tile that opens the picker.
                Button(action: choosePhoto) {
                    RoundedRectangle(cornerRadius: 16, style: .continuous)
                        .strokeBorder(Palette.rule2, style: StrokeStyle(lineWidth: 1.5, dash: [6, 5]))
                        .overlay {
                            if importing { ProgressView() } else { GlyphIcon(.plus, size: 22).foregroundStyle(Palette.ink2) }
                        }
                        .frame(width: 84, height: Self.tileHeight)
                        .contentShape(RoundedRectangle(cornerRadius: 16, style: .continuous))
                }
                .buttonStyle(PressDimsStyle())
                .disabled(importing)
                .accessibilityLabel(wallpaper.title)
                .accessibilityIdentifier("ownWallpaper")
            } else {
                Button {
                    guard wallpaper != choice.image else { return }
                    feedback.play(.selection)
                    update { $0 = $0.with(image: wallpaper) }
                } label: {
                    PickedTile(on: on, radius: 16) {
                        TilePicture(image: thumb ?? WallpaperArt(choice: .of(wallpaper), dark: art.dark, photo: photo).base.ready()?.image)
                    }
                    .frame(width: 84, height: Self.tileHeight)
                }
                .buttonStyle(PressDimsStyle())
                .accessibilityLabel(wallpaper.title)
                .accessibilityAddTraits(on ? [.isButton, .isSelected] : .isButton)
                .accessibilityIdentifier("wallpaper:\(wallpaper.rawValue)")
                .overlay(alignment: .topTrailing) {
                    if wallpaper == .own {
                        // Your photo: another one in its place.
                        Button(action: choosePhoto) {
                            Group {
                                if importing { ProgressView().controlSize(.mini) } else { GlyphIcon(.pencil, size: 14) }
                            }
                            .foregroundStyle(Palette.ink2)
                            .frame(width: 26, height: 26)
                            .background(Palette.paperRaised, in: Circle())
                            .contentShape(Circle())
                        }
                        .buttonStyle(PressDimsStyle())
                        .disabled(importing)
                        .padding(6)
                        .accessibilityLabel("Replace photo")
                        .accessibilityIdentifier("ownWallpaper")
                    }
                }
            }
            Text(wallpaper.title)
                .font(RFont.sans(12.5, on ? .semibold : .regular))
                .foregroundStyle(on ? Palette.ink : Palette.ink2)
                .lineLimit(1)
                .minimumScaleFactor(0.8)
                .accessibilityHidden(true)
        }
        .frame(width: 84)
    }

    /// A tile's height: Android's 0.77 aspect on an 84-pt tile.
    private static let tileHeight: CGFloat = 84 / 0.77

    private func choosePhoto() {
        feedback.play(.open)
        choosingPhoto = true
    }

    // MARK: Effect

    /// The image with each effect, small, in five equal tiles across; the chosen one ringed.
    private func effectRow(art: WallpaperArt, thumbs: [WallpaperEffect: UIImage]) -> some View {
        HStack(alignment: .top, spacing: 8) {
            ForEach(WallpaperEffect.allCases) { effect in
                let on = effect == art.choice.effect
                VStack(spacing: 4) {
                    Button {
                        guard choice.effect != effect else { return }
                        feedback.play(.selection)
                        update { $0.effect = effect }
                    } label: {
                        PickedTile(on: on, radius: 14) { TilePicture(image: thumbs[effect]) }
                            .aspectRatio(0.77, contentMode: .fit)
                    }
                    .buttonStyle(PressDimsStyle())
                    .accessibilityLabel(effect.title)
                    .accessibilityAddTraits(on ? [.isButton, .isSelected] : .isButton)
                    .accessibilityIdentifier("effect:\(effect.rawValue)")
                    Text(effect.title)
                        .font(RFont.sans(12, on ? .semibold : .regular))
                        .foregroundStyle(on ? Palette.ink : Palette.ink2)
                        .lineLimit(1)
                        .minimumScaleFactor(0.8)
                        .accessibilityHidden(true)
                }
                .frame(maxWidth: .infinity)
            }
        }
        .padding(.horizontal, 16)
    }

    // MARK: Changes

    private func update(_ change: (inout WallpaperChoice) -> Void) {
        if scheme == .dark { change(&dark) } else { change(&light) }
    }

    /// While the slider is held the fade only moves the preview; a step without a hold (VoiceOver) is kept at once.
    private func setFade(_ value: Double) {
        if sliding { heldFade = value } else { update { $0.fade = value } }
    }

    /// Keeps a picked photo for the theme as it is (upright, at most 2,000 px) and chooses it, keeping the effect.
    /// A photo needs a fade into the page: with none set, it opens with the default (Android's `withNewPhoto`).
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
        } catch {
            feedback.play(.error)
            self.error = error.userMessage
        }
    }
}

/// A quiet label over a row of choices (Android's `Label`).
private struct WallpaperLabel: View {
    var text: String

    init(_ text: String) { self.text = text }

    var body: some View {
        Text(text)
            .font(RFont.sans(12.5, .semibold))
            .tracking(12.5 * 0.015)
            .foregroundStyle(Palette.inkMeta)
            .lineLimit(1)
            .padding(.horizontal, 20)
            .padding(.top, 22)
            .padding(.bottom, 8)
            .accessibilityAddTraits(.isHeader)
    }
}

/// A choice tile (Android's `Picked`): when `on`, a 2-pt violet ring on its edge with the picture 3 pt inside it and a
/// check at the bottom right; otherwise a hairline ring. The tile takes the size it is given.
private struct PickedTile<Content: View>: View {
    var on: Bool
    var radius: CGFloat
    @ViewBuilder var content: Content

    var body: some View {
        let inner = on ? radius - 3 : radius
        let shape = RoundedRectangle(cornerRadius: radius, style: .continuous)
        content
            .clipShape(RoundedRectangle(cornerRadius: inner, style: .continuous))
            .padding(on ? 3 : 0)
            .overlay(shape.strokeBorder(on ? Palette.violet : Palette.rule, lineWidth: on ? 2 : 1))
            .overlay(alignment: .bottomTrailing) {
                if on {
                    GlyphIcon(.check, size: 14)
                        .foregroundStyle(.white)
                        .frame(width: 20, height: 20)
                        .background(Palette.violetFill, in: Circle())
                        .padding(6)
                }
            }
            .contentShape(shape)
    }
}

/// A picture filling its tile, cropped from its top, on sunken paper until it is ready.
private struct TilePicture: View {
    var image: UIImage?

    var body: some View {
        Palette.paperSunken
            .overlay(alignment: .top) {
                if let image {
                    Image(uiImage: image).resizable().interpolation(.medium).scaledToFill()
                }
            }
            .clipped()
            .accessibilityHidden(true)
    }
}

/// The page as it would look (Next, Settings 04c; Android's `Preview`): the picture at the top under the title's
/// shade, panes under it, in the chosen theme's paper. 200 × 256.
private struct PreviewCard: View {
    var image: UIImage?
    var dark: Bool

    var body: some View {
        ZStack(alignment: .topLeading) {
            Palette.paper
            if let image {
                Image(uiImage: image).resizable().interpolation(.medium).scaledToFill()
                    .frame(width: 200, height: 256, alignment: .top)
                    .clipped()
            }
            LinearGradient(colors: [dark ? Color.black.opacity(0.55) : Palette.paper.opacity(0.75), .clear], startPoint: .top, endPoint: .bottom)
                .frame(height: 60)
            Text("Activity")
                .font(RFont.fixedDisplay(24))
                .tracking(RFont.displayTracking(24))
                .foregroundStyle(dark ? .white : Palette.ink)
                .lineLimit(1)
                .padding(.leading, 14)
                .padding(.top, 14)
            VStack(spacing: 8) {
                ForEach(0..<4, id: \.self) { _ in
                    RoundedRectangle(cornerRadius: 10, style: .continuous).fill(Palette.paperRaised).frame(height: 26)
                }
            }
            .padding(10)
            .frame(maxHeight: .infinity, alignment: .bottom)
        }
        .frame(width: 200, height: 256)
        .clipShape(RoundedRectangle(cornerRadius: 24, style: .continuous))
        .environment(\.colorScheme, dark ? .dark : .light)
        .accessibilityHidden(true)
        .accessibilityIdentifier("wallpaperPreview")
    }
}

/// An image cut to a rounded square, its top showing: the Settings row's tile.
struct WallpaperThumb: View {
    var image: UIImage?
    var side: CGFloat
    var radius: CGFloat

    var body: some View {
        TilePicture(image: image)
            .frame(width: side, height: side)
            .clipShape(RoundedRectangle(cornerRadius: radius, style: .continuous))
    }
}

/// The theme's wallpaper as it is drawn (its effect and fade), as a small square: the Settings row's tile. While new art
/// renders, the last stays, else the picture it is drawn on.
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
        WallpaperThumb(image: (art.ready() ?? drawn.flatMap { $0.art.dark == isDark ? $0 : nil } ?? art.base.ready())?.image, side: side, radius: radius)
            .task(id: art) {
                if let made = await art.drawn() { drawn = made }
            }
    }
}
