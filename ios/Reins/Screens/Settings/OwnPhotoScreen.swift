import PhotosUI
import SwiftUI
import UIKit

/// Settings > Wallpaper > Your photo (the Settings board, 04c), quiet: pick a photo, see it behind a small Activity,
/// pick an effect and how far it fades, then use it for dark or for light. Everything is done on the phone and the fade
/// is baked into the saved image (`OwnWallpaper.save`); the choice is kept under `Wallpaper.key(scheme)`.
struct OwnPhotoScreen: View {
    var dark: Bool
    @Environment(AppModel.self) private var model
    @Environment(\.feedback) private var feedback
    @Environment(\.dismiss) private var dismiss
    @State private var item: PhotosPickerItem?
    @State private var source: CGImage?
    @State private var effect: WallpaperEffect = .dither
    @State private var fade = WallpaperRender.defaultFade
    @State private var preview: CGImage?
    @State private var thumbs: [WallpaperEffect: CGImage] = [:]
    @State private var saving = false
    @State private var error: String?

    private var scheme: ColorScheme { dark ? .dark : .light }

    var body: some View {
        QuietPage("Your photo") {
            PhotosPicker(selection: $item, matching: .images, photoLibrary: .shared()) {
                PhonePreview(image: preview, dark: dark, empty: source == nil)
            }
            .buttonStyle(PressDimsStyle())
            .frame(maxWidth: .infinity)
            .padding(.top, 14)
            .accessibilityLabel(source == nil ? "Pick a photo" : "Pick another photo")
            .accessibilityIdentifier("pickPhoto")
            if source != nil {
                HStack(alignment: .top, spacing: 0) {
                    ForEach(WallpaperEffect.allCases) { e in
                        EffectChoice(effect: e, thumb: thumbs[e], selected: effect == e) {
                            guard effect != e else { return }
                            feedback.play(.selection)
                            effect = e
                        }
                        if e != WallpaperEffect.allCases.last { Spacer(minLength: 4) }
                    }
                }
                .padding(.horizontal, 16)
                .padding(.top, 18)
                HStack(spacing: 12) {
                    GlyphIcon(.appearance, size: 16).foregroundStyle(Palette.inkMeta).accessibilityHidden(true)
                    Slider(value: $fade, in: 0...1) { editing in
                        if !editing { feedback.play(.selection) }
                    }
                    .tint(Palette.violet)
                    .accessibilityLabel("Fade")
                    .accessibilityIdentifier("wallpaperFade")
                    Text("Fade").font(RFont.sans(13)).foregroundStyle(Palette.ink2).accessibilityHidden(true)
                }
                .padding(.horizontal, 24)
                .padding(.top, 18)
            }
            if let error { QuietBanner(text: error) }
        } footer: {
            QuietFooter {
                Button(action: save) {
                    HStack(spacing: 8) {
                        if saving { ProgressView().tint(Palette.onInverse) } else { GlyphIcon(.check, size: 18) }
                        Text(dark ? "Use for dark" : "Use for light")
                    }
                }
                .buttonStyle(SignalButtonStyle(.inverse))
                .disabled(source == nil || saving)
                .accessibilityIdentifier("useWallpaper")
            }
        }
        .onChange(of: item) { _, picked in
            guard let picked else { return }
            Task { await load(picked) }
        }
        .task(id: RenderKey(source: source.map(ObjectIdentifier.init), effect: effect, fade: fade)) {
            await renderPreview()
        }
        .task(id: source.map(ObjectIdentifier.init)) { await renderThumbs() }
        #if DEBUG
        .onAppear {
            // `-demoOwnPhoto`: start from a shipped image, so the page can be looked at without a photo library.
            if source == nil, ProcessInfo.processInfo.arguments.contains("-demoOwnPhoto") {
                source = UIImage(named: "wallpaper-eyes")?.cgImage
            }
        }
        #endif
    }

    private struct RenderKey: Equatable {
        var source: ObjectIdentifier?
        var effect: WallpaperEffect
        var fade: Double
    }

    private func load(_ picked: PhotosPickerItem) async {
        error = nil
        do {
            guard let data = try await picked.loadTransferable(type: Data.self), let image = UIImage(data: data) else {
                error = "That photo could not be opened."
                return
            }
            source = Self.upright(image, maxSide: 2_000)
        } catch {
            self.error = error.userMessage
        }
    }

    /// The photo the right way up and no bigger than `maxSide`, as the effects expect it.
    static func upright(_ image: UIImage, maxSide: CGFloat) -> CGImage? {
        let scale = min(1, maxSide / max(image.size.width, image.size.height))
        let size = CGSize(width: image.size.width * scale, height: image.size.height * scale)
        let format = UIGraphicsImageRendererFormat()
        format.scale = 1
        format.opaque = true
        return UIGraphicsImageRenderer(size: size, format: format).image { _ in image.draw(in: CGRect(origin: .zero, size: size)) }.cgImage
    }

    private func renderPreview() async {
        guard let source else { preview = nil; return }
        // Let a dragged slider settle before drawing the full size.
        try? await Task.sleep(for: .milliseconds(60))
        guard !Task.isCancelled else { return }
        let (effect, fade, dark) = (effect, fade, dark)
        let size = CGSize(width: 440, height: 564)
        let image = await Task.detached(priority: .userInitiated) {
            WallpaperRender.render(source, effect: effect, fade: fade, dark: dark, size: size)
        }.value
        if !Task.isCancelled { preview = image }
    }

    private func renderThumbs() async {
        guard let source else { thumbs = [:]; return }
        let dark = dark
        let made = await Task.detached(priority: .utility) {
            var out: [WallpaperEffect: CGImage] = [:]
            for e in WallpaperEffect.allCases {
                out[e] = WallpaperRender.render(source, effect: e, fade: 0, dark: dark, size: CGSize(width: 156, height: 200))
            }
            return out
        }.value
        if !Task.isCancelled { thumbs = made }
    }

    private func save() {
        guard let source, !saving else { return }
        saving = true
        error = nil
        let (effect, fade, dark, scheme) = (effect, fade, dark, scheme)
        Task {
            let image = await Task.detached(priority: .userInitiated) {
                WallpaperRender.render(source, effect: effect, fade: fade, dark: dark)
            }.value
            do {
                guard let image else { throw CocoaError(.fileWriteUnknown) }
                try OwnWallpaper.save(UIImage(cgImage: image), scheme)
                AppGroup.defaults.set(Wallpaper.own.rawValue, forKey: Wallpaper.key(scheme))
                feedback.play(.selection)
                saving = false
                dismiss()
            } catch {
                feedback.play(.error)
                self.error = "The wallpaper could not be saved."
                saving = false
            }
        }
    }
}

/// The photo behind a small Activity, in a phone's frame: what the page will look like.
private struct PhonePreview: View {
    var image: CGImage?
    var dark: Bool
    var empty: Bool

    var body: some View {
        let shape = RoundedRectangle(cornerRadius: 30, style: .continuous)
        let paper = dark ? Color(uiColor: UIColor(hex: 0x121215)) : Color(uiColor: UIColor(hex: 0xF6F6F8))
        let bar = dark ? Color(uiColor: UIColor(hex: 0x1C1C21)) : Color.white
        ZStack(alignment: .topLeading) {
            paper
            if let image {
                Image(decorative: image, scale: 1).resizable().interpolation(.medium).frame(width: 220, height: 282)
            }
            if empty {
                VStack(spacing: 8) {
                    GlyphIcon(.plus, size: 24)
                    Text("Pick a photo").font(RFont.sans(14, .semibold))
                }
                .foregroundStyle(dark ? Color.white.opacity(0.7) : Palette.ink2)
                .frame(width: 220, height: 282)
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
    }
}

/// An effect as a small square of the photo with it applied, its name under it; the chosen one is ringed in violet.
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
