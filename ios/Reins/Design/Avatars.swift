import SwiftUI

// Avatars. The API is fixed; the drawing is a placeholder until the provider logos and the blobatar port land
// (Android: design/Avatars.kt, design/blobatar/Blobatar.kt).

/// An AI connection: the logo of the provider picked for it (`pick`, a provider key such as "claude"), the one its
/// name suggests when nothing was picked, or a blobatar seeded by the name.
struct ConnectionAvatar: View {
    var label: String
    var pick: String?
    var size: CGFloat = 40

    var body: some View {
        BlobAvatar(seed: label, size: size)
    }
}

/// A figure drawn from a name, the same one for the same name everywhere.
struct BlobAvatar: View {
    var seed: String
    var size: CGFloat = 40

    var body: some View {
        Circle()
            .fill(Palette.accentSoft)
            .overlay(
                Text(String(seed.trimmingCharacters(in: .whitespaces).prefix(1)).uppercased())
                    .font(RFont.fixedSans(size * 0.42, .semibold))
                    .foregroundStyle(Palette.accent)
            )
            .frame(width: size, height: size)
            .accessibilityHidden(true)
    }
}

/// An integration's logo ("gmail", "github", "mcp:<id>").
struct ServiceAvatar: View {
    var service: String
    var size: CGFloat = 40

    var body: some View {
        let key = service.hasPrefix(mcpPrefix) ? "mcp" : service
        RoundedRectangle(cornerRadius: size * 0.28, style: .continuous)
            .fill(Palette.elevated)
            .overlay(
                Image("service-\(key)")
                    .resizable()
                    .scaledToFit()
                    .padding(size * 0.2)
            )
            .overlay(RoundedRectangle(cornerRadius: size * 0.28, style: .continuous).strokeBorder(Palette.hairline, lineWidth: 0.5))
            .frame(width: size, height: size)
            .accessibilityHidden(true)
    }
}
