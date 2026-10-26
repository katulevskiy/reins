import SwiftUI

/// Settings > Wallpaper (the Settings board, 04b), quiet: one wallpaper for dark and one for light, picked from the eight
/// shipped ones or your own photo. The choice is kept under `Wallpaper.key(scheme)` in the app group, where
/// `WallpaperLayer` reads it.
struct WallpaperScreen: View {
    @Environment(AppModel.self) private var model
    @Environment(\.colorScheme) private var systemScheme
    @Environment(\.feedback) private var feedback
    @AppStorage(Wallpaper.key(.dark), store: AppGroup.defaults) private var dark = Wallpaper.defaultFor(.dark).rawValue
    @AppStorage(Wallpaper.key(.light), store: AppGroup.defaults) private var light = Wallpaper.defaultFor(.light).rawValue
    /// Which theme's wallpaper is being picked: Dark first, as the board opens.
    @State private var picking: ColorScheme? = .dark
    /// Bumped when the page shows again, so a photo saved meanwhile is drawn.
    @State private var appeared = 0

    private var scheme: ColorScheme { picking ?? systemScheme }

    private var chosen: Wallpaper {
        Wallpaper(rawValue: scheme == .dark ? dark : light) ?? .defaultFor(scheme)
    }

    private let columns = Array(repeating: GridItem(.flexible(minimum: 80, maximum: 104), spacing: 12, alignment: .top), count: 3)

    var body: some View {
        QuietPage("Wallpaper") {
            SchemeSegments(scheme: Binding(get: { scheme }, set: { picking = $0 }))
                .padding(.horizontal, 20)
                .padding(.top, 4)
            LazyVGrid(columns: columns, alignment: .leading, spacing: 14) {
                ForEach(Wallpaper.shipped) { wallpaper in
                    tile(wallpaper)
                }
                if OwnWallpaper.load(scheme) != nil {
                    tile(.own).id("own-\(appeared)")
                }
                OwnPhotoTile {
                    model.show(.ownWallpaper(dark: scheme == .dark))
                }
            }
            .padding(.horizontal, 20)
            .padding(.top, 18)
        }
        .onAppear { appeared += 1 }
    }

    private func tile(_ wallpaper: Wallpaper) -> some View {
        WallpaperChoice(wallpaper: wallpaper, scheme: scheme, selected: chosen == wallpaper) {
            guard chosen != wallpaper else { return }
            feedback.play(.selection)
            if scheme == .dark { dark = wallpaper.rawValue } else { light = wallpaper.rawValue }
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

/// A wallpaper as a small portrait, its top showing (the board's `thumb`): ringed in violet with a check when chosen,
/// its name under it.
private struct WallpaperChoice: View {
    var wallpaper: Wallpaper
    var scheme: ColorScheme
    var selected: Bool
    var action: () -> Void

    var body: some View {
        Button(action: action) {
            VStack(spacing: 6) {
                WallpaperThumb(wallpaper: wallpaper, scheme: scheme, side: nil, radius: 16)
                    .aspectRatio(104 / 135, contentMode: .fit)
                    .overlay {
                        if selected {
                            RoundedRectangle(cornerRadius: 18.5, style: .continuous).strokeBorder(Palette.violet, lineWidth: 2.5).padding(-2.5)
                        } else {
                            RoundedRectangle(cornerRadius: 16, style: .continuous).strokeBorder(Palette.rule, lineWidth: 1)
                        }
                    }
                    .overlay(alignment: .bottomTrailing) {
                        if selected {
                            GlyphIcon(.check, size: 13)
                                .foregroundStyle(.white)
                                .frame(width: 22, height: 22)
                                .background(Palette.violetFill, in: Circle())
                                .padding(6)
                        }
                    }
                Text(wallpaper.title)
                    .font(RFont.sans(12.5))
                    .foregroundStyle(Palette.ink2)
                    .lineLimit(1)
                    .minimumScaleFactor(0.8)
            }
            .contentShape(Rectangle())
        }
        .buttonStyle(PressDimsStyle())
        .accessibilityLabel(wallpaper.title)
        .accessibilityAddTraits(selected ? [.isButton, .isSelected] : .isButton)
        .accessibilityIdentifier("wallpaper:\(wallpaper.rawValue)")
    }
}

/// The dashed tile that opens "Your photo".
private struct OwnPhotoTile: View {
    var action: () -> Void
    @Environment(\.feedback) private var feedback

    var body: some View {
        Button {
            action()
            feedback.defaultTap()
        } label: {
            RoundedRectangle(cornerRadius: 16, style: .continuous)
                .strokeBorder(Palette.rule2, style: StrokeStyle(lineWidth: 1.5, dash: [4, 3]))
                .aspectRatio(104 / 135, contentMode: .fit)
                .overlay {
                    VStack(spacing: 6) {
                        GlyphIcon(.plus, size: 22)
                        Text("Your photo").font(RFont.sans(13, .semibold)).multilineTextAlignment(.center)
                    }
                    .foregroundStyle(Palette.ink2)
                }
                .contentShape(Rectangle())
        }
        .buttonStyle(PressDimsStyle())
        .accessibilityLabel("Your photo")
        .accessibilityIdentifier("ownWallpaper")
    }
}

/// A wallpaper's image cut to a rounded tile, its top showing; `side` fixes a square (the Settings row's 32 pt), nil
/// fills what it is given.
struct WallpaperThumb: View {
    var wallpaper: Wallpaper
    var scheme: ColorScheme
    var side: CGFloat?
    var radius: CGFloat

    var body: some View {
        let image = wallpaper.image(scheme) ?? Wallpaper.defaultFor(scheme).image(scheme)
        Color.clear
            .frame(width: side, height: side)
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
