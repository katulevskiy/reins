import SwiftUI
import UIKit

// Avatars (the Android app's design/Avatars.kt): provider logos for AI connections, platform logos for computers,
// blobatars when no logo fits, and service logos for integrations, all on the same round plate.

// `Provider`, `Platform`, `AvatarLook` and the logo tables are in Shared/Design/Providers.swift, for the widgets too.

/// How a connection looks, from what the core says of it (pure, unit-tested).
enum ConnectionLook {
    /// The icon it shows: the one picked, else what Auto shows (`autoIcon`). Nil only for older data without an
    /// `autoIcon`, which then follows the older rule (the provider its name suggests, else its blobatar).
    static func shownIcon(_ c: ConnectionView) -> String? {
        c.icon ?? (c.autoIcon.isEmpty ? nil : c.autoIcon)
    }

    /// The blobatar it keeps, if the core sent one.
    static func blobatar(_ c: ConnectionView) -> BlobatarLook? {
        c.blobatar.map { BlobatarLook(seed: $0.seed, shape: Int($0.shape), colour: Int($0.colour)) }
    }

    /// What its avatar draws everywhere.
    static func look(_ c: ConnectionView) -> AvatarLook {
        .of(label: untrusted(c.label), icon: shownIcon(c), blobatar: blobatar(c))
    }

    /// What "Auto" draws for it.
    static func auto(_ c: ConnectionView) -> AvatarLook {
        .of(label: untrusted(c.label), icon: c.autoIcon.isEmpty ? nil : c.autoIcon, blobatar: blobatar(c))
    }

    /// Its blobatar, whatever it shows.
    static func blob(_ c: ConnectionView) -> AvatarLook {
        .of(label: untrusted(c.label), icon: Providers.blob, blobatar: blobatar(c))
    }

    /// Whether it shows its blobatar now (customising the blobatar then needs no new icon).
    static func showsBlob(_ c: ConnectionView) -> Bool { look(c).isBlob }
}

/// A connection's avatar: a provider's logo, a computer's platform logo, or its blobatar.
struct ConnectionAvatar: View {
    var look: AvatarLook
    var size: CGFloat = 40

    init(look: AvatarLook, size: CGFloat = 40) {
        self.look = look
        self.size = size
    }

    /// A connection as it shows everywhere: the icon picked for it, else what Auto shows, with the blobatar it keeps.
    init(connection: ConnectionView, size: CGFloat = 40) {
        self.init(look: ConnectionLook.look(connection), size: size)
    }

    /// For callers with only a name (an Activity entry of a connection that is gone): the icon picked (`pick`, a
    /// provider's or a platform's key, or "blob"), the provider the name suggests when nothing was picked, or a
    /// blobatar seeded by the name.
    init(label: String, pick: String?, size: CGFloat = 40) {
        self.init(look: .of(label: label, icon: pick), size: size)
    }

    var body: some View {
        switch look {
        case let .provider(provider): ProviderAvatar(provider: provider, size: size)
        case let .platform(platform): PlatformAvatar(platform: platform, size: size)
        case let .blob(seed, shape, colour): BlobAvatar(seed: seed, shape: shape, colour: colour, size: size)
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

/// A computer's platform logo (Apple, Windows, Linux) on the same plate as the provider logos.
struct PlatformAvatar: View {
    var platform: Platform
    var size: CGFloat = 40

    var body: some View {
        AvatarPlate(size: size) {
            Image(platform.asset)
                .renderingMode(.template)
                .resizable()
                .scaledToFit()
                .foregroundStyle(platform.brand.map { Color(uiColor: UIColor(hex: $0)) } ?? Palette.text)
                .frame(width: size * 0.52, height: size * 0.52)
        }
        .accessibilityHidden(true)
    }
}

/// A blobatar: the deterministic figure the library draws for `seed`, the same one for the same name everywhere, on
/// the same round plate as the logos; `shape` and `colour` are the ones a connection keeps.
struct BlobAvatar: View {
    var seed: String
    var shape: Int? = nil
    var colour: Int? = nil
    var size: CGFloat = 40

    var body: some View {
        let marks = BlobatarDrawing.marks(for: seed, shape: shape, colour: colour)
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

/// Choosing a connection's icon (Android: the ICON grid on ConnectionDetailScreen): Auto (drawn as what it resolves
/// to), its blobatar, for a computer the platform logos, then any provider's logo. `selection` is the stored pick (nil =
/// Auto, "blob", a platform's or a provider's key); `onPick` gets the new one. Place `ProviderPicker.footnote` under it.
struct ProviderPicker: View {
    /// What Auto draws, and the blobatar.
    var auto: AvatarLook
    var blob: AvatarLook
    /// A computer: the platform logos are offered too.
    var platforms: Bool
    var selection: String?
    var size: CGFloat = 52
    var onPick: (String?) -> Void

    @Environment(\.feedback) private var feedback

    static let footnote = "Auto shows a computer's platform or a known AI's logo, else a blobatar."

    /// For a connection.
    init(connection: ConnectionView, size: CGFloat = 52, onPick: @escaping (String?) -> Void) {
        auto = ConnectionLook.auto(connection)
        blob = ConnectionLook.blob(connection)
        platforms = connection.keyFingerprint != nil
        selection = connection.icon
        self.size = size
        self.onPick = onPick
    }

    /// From a name only: Auto and Blobatar draw from it.
    init(label: String, selection: String?, size: CGFloat = 52, onPick: @escaping (String?) -> Void) {
        auto = .of(label: label, icon: nil)
        blob = .of(label: label, icon: Providers.blob)
        platforms = false
        self.selection = selection
        self.size = size
        self.onPick = onPick
    }

    private struct Choice: Identifiable {
        let id: String
        let name: String
        let pick: String?
        let look: AvatarLook
    }

    private var choices: [Choice] {
        [Choice(id: "auto", name: "Auto", pick: nil, look: auto),
         Choice(id: Providers.blob, name: "Blobatar", pick: Providers.blob, look: blob)]
            + (platforms ? Platforms.all.map { Choice(id: $0.key, name: $0.name, pick: $0.key, look: .platform($0)) } : [])
            + Providers.all.map { Choice(id: $0.key, name: $0.name, pick: $0.key, look: .provider($0)) }
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
                        ConnectionAvatar(look: choice.look, size: size)
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

/// A connection's blobatar, changed: its ten shapes (in its colour), its twelve colours, and Shuffle. `shape` and
/// `colour` are the kept ones (nil: none kept yet, nothing ringed).
struct BlobatarEditor: View {
    var seed: String
    var shape: Int?
    var colour: Int?
    var onShape: (Int) -> Void
    var onColour: (Int) -> Void
    var onShuffle: () -> Void

    @Environment(\.feedback) private var feedback

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            HStack {
                GroupHeader("Blobatar")
                Spacer()
                Button {
                    feedback.play(.tap)
                    onShuffle()
                } label: {
                    Label("Shuffle", systemImage: "shuffle")
                }
                .buttonStyle(SignalButtonStyle(.soft, size: .small, fill: false))
                .accessibilityLabel("Shuffle")
                .accessibilityIdentifier("shuffleBlobatar")
            }
            ScrollView(.horizontal, showsIndicators: false) {
                HStack(spacing: 8) {
                    ForEach(BlobatarPalette.shapes.indices, id: \.self) { i in
                        ring(selected: i == shape) {
                            BlobAvatar(seed: seed, shape: i, colour: colour, size: 40)
                        }
                        .pick(i == shape, label: "Shape \(i + 1)", id: "shape:\(i)") {
                            feedback.play(.selection)
                            onShape(i)
                        }
                    }
                }
                .padding(.vertical, 2)
            }
            ScrollView(.horizontal, showsIndicators: false) {
                HStack(spacing: 8) {
                    ForEach(BlobatarPalette.colours.indices, id: \.self) { i in
                        ring(selected: i == colour) {
                            Circle()
                                .fill(BlobatarDrawing.headColor(i))
                                .frame(width: 30, height: 30)
                        }
                        .pick(i == colour, label: BlobatarPalette.colours[i].title, id: "colour:\(i)") {
                            feedback.play(.selection)
                            onColour(i)
                        }
                    }
                }
                .padding(.vertical, 2)
            }
        }
    }

    private func ring(selected: Bool, @ViewBuilder _ content: () -> some View) -> some View {
        content()
            .padding(3)
            .overlay(Circle().strokeBorder(selected ? Palette.accent : .clear, lineWidth: 2.5))
    }
}

private extension View {
    /// One choice of the blobatar editor: a button with its name, marked selected.
    func pick(_ selected: Bool, label: String, id: String, action: @escaping () -> Void) -> some View {
        Button(action: action) { self.contentShape(Circle()) }
            .buttonStyle(.plain)
            .accessibilityLabel(label)
            .accessibilityAddTraits(selected ? .isSelected : [])
            .accessibilityIdentifier(id)
    }
}
