import SwiftUI
import UIKit

// Avatars (the Android app's design/Avatars.kt): provider logos for AI connections, blobatars drawn from a name when
// no logo fits, and service logos for integrations, all on the same round plate.

// `Provider` and `Providers` (the logo table) are in Shared/Design/Providers.swift, for the widgets too.

/// An AI connection: the logo of the provider picked for it (`pick`, a provider key such as "claude", or "blob"), the
/// one its name suggests when nothing was picked, or a blobatar seeded by the name.
struct ConnectionAvatar: View {
    var label: String
    var pick: String?
    var size: CGFloat = 40

    var body: some View {
        if let provider = Providers.resolve(label: label, pick: pick) {
            ProviderAvatar(provider: provider, size: size)
        } else {
            BlobAvatar(seed: label, size: size)
        }
    }
}

/// The round plate every avatar sits on, so provider logos, blobatars and services look alike.
struct AvatarPlate<Content: View>: View {
    var size: CGFloat
    @ViewBuilder var content: Content

    static var fill: Color { Palette.dynamic(0xFFFFFF, 0x1D1D21) }

    var body: some View {
        ZStack { content }
            .frame(width: size, height: size)
            .background(Self.fill, in: Circle())
            .overlay(Circle().strokeBorder(Palette.hairline, lineWidth: 1))
            .clipShape(Circle())
    }
}

struct ProviderAvatar: View {
    var provider: Provider
    var size: CGFloat = 40

    var body: some View {
        AvatarPlate(size: size) {
            Image(provider.asset)
                .renderingMode(provider.mono ? .template : .original)
                .resizable()
                .scaledToFit()
                .foregroundStyle(Palette.text)
                .frame(width: size * 0.56, height: size * 0.56)
        }
        .accessibilityHidden(true)
    }
}

/// A blobatar: the deterministic figure the library draws for `seed`, the same one for the same name everywhere, on
/// the same round plate as the logos.
struct BlobAvatar: View {
    var seed: String
    var size: CGFloat = 40

    var body: some View {
        let marks = BlobatarDrawing.marks(for: seed)
        AvatarPlate(size: size) {
            Canvas { context, canvas in
                let scale = min(canvas.width, canvas.height) / 100
                context.scaleBy(x: scale, y: scale)
                for mark in marks { context.fill(mark.path, with: .color(mark.color)) }
            }
            .frame(width: size * 0.92, height: size * 0.92)
        }
        .accessibilityHidden(true)
    }
}

/// An integration's logo ("gmail", "github", "mcp:<id>") on a plate, in its brand colour.
struct ServiceAvatar: View {
    var service: String
    var size: CGFloat = 40

    var body: some View {
        let key = service.hasPrefix(mcpPrefix) ? "mcp" : service
        AvatarPlate(size: size) {
            if UIImage(named: "service-\(key)") != nil {
                Image("service-\(key)")
                    .renderingMode(.template)
                    .resizable()
                    .scaledToFit()
                    .foregroundStyle(serviceColor(key))
                    .frame(width: size * 0.5, height: size * 0.5)
            } else {
                Image(systemName: "link")
                    .font(.system(size: size * 0.4, weight: .medium))
                    .foregroundStyle(Palette.secondary)
            }
        }
        .accessibilityHidden(true)
    }
}

/// The brand colour of a service's mark.
func serviceColor(_ service: String) -> Color {
    switch service {
    case "gmail": Color(uiColor: UIColor(hex: 0xEA4335))
    case "telegram": Color(uiColor: UIColor(hex: 0x26A5E4))
    case "github": Palette.text
    case "gitlab": Color(uiColor: UIColor(hex: 0xFC6D26))
    case "codeberg": Color(uiColor: UIColor(hex: 0x2185D0))
    case "bitbucket": Color(uiColor: UIColor(hex: 0x0052CC))
    case "mcp": Palette.accent
    case "gcalendar", "gcontacts": Color(uiColor: UIColor(hex: 0x4285F4))
    case "device_calendar": Color(uiColor: UIColor(hex: 0x34A853))
    case "device_contacts": Color(uiColor: UIColor(hex: 0xF57C00))
    case "sms": Color(uiColor: UIColor(hex: 0x00A884))
    case "vault": Color(uiColor: UIColor(hex: 0x175DDC))
    default: Palette.secondary
    }
}

/// Choosing a connection's icon (Android: the ICON grid on ConnectionDetailScreen): Auto (from the name), a blobatar,
/// or any provider's logo. `selection` is the stored pick (nil = Auto, "blob", or a provider key); `onPick` gets the
/// new one. Place `ProviderPicker.footnote` under it.
struct ProviderPicker: View {
    /// The connection's name: what Auto and Blobatar draw from.
    var label: String
    var selection: String?
    var size: CGFloat = 52
    var onPick: (String?) -> Void

    @Environment(\.feedback) private var feedback

    static let footnote = "Auto picks a known AI from the name, or draws a blobatar for it."

    private struct Choice: Identifiable {
        let id: String
        let name: String
        let pick: String?
        let avatarLabel: String
    }

    private var choices: [Choice] {
        [Choice(id: "auto", name: "Auto", pick: nil, avatarLabel: label),
         Choice(id: Providers.blob, name: "Blobatar", pick: Providers.blob, avatarLabel: label)]
            + Providers.all.map { Choice(id: $0.key, name: $0.name, pick: $0.key, avatarLabel: $0.name) }
    }

    var body: some View {
        LazyVGrid(columns: [GridItem(.adaptive(minimum: size + 16), spacing: 12)], spacing: 14) {
            ForEach(choices) { choice in
                let selected = choice.pick == selection
                Button {
                    guard !selected else { return }
                    feedback.play(.selection)
                    onPick(choice.pick)
                } label: {
                    VStack(spacing: 5) {
                        ConnectionAvatar(label: choice.avatarLabel, pick: choice.pick, size: size)
                            .padding(3)
                            .overlay(Circle().strokeBorder(selected ? Palette.accent : .clear, lineWidth: 2.5))
                        Text(choice.name)
                            .font(RFont.sans(12, selected ? .semibold : .regular))
                            .foregroundStyle(selected ? Palette.accent : Palette.secondary)
                            .lineLimit(1)
                            .minimumScaleFactor(0.8)
                    }
                    .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .accessibilityLabel(choice.name)
                .accessibilityAddTraits(selected ? .isSelected : [])
                .accessibilityIdentifier("icon:\(choice.id)")
            }
        }
    }
}
