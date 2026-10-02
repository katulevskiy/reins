import SwiftUI
import UIKit

// Avatars (the Android app's design/Avatars.kt): provider logos for AI connections, blobatars drawn from a name when
// no logo fits, and service logos for integrations, all on the same round plate.

/// An AI provider the user can pick as a connection's icon. Its logo is `provider-<key>` in Brand.xcassets (see
/// BRAND-NOTICE.md); `mono` logos are single-colour and take the text colour, the others keep their brand colours.
struct Provider: Identifiable, Equatable {
    let key: String
    let name: String
    let mono: Bool
    /// Words in a connection's name that suggest this provider.
    let words: [String]

    var id: String { key }
    var asset: String { "provider-\(key)" }
}

enum Providers {
    static let all: [Provider] = [
        Provider(key: "claude", name: "Claude", mono: false, words: ["claude", "anthropic"]),
        Provider(key: "openai", name: "ChatGPT", mono: true, words: ["chatgpt", "gpt", "openai", "codex"]),
        Provider(key: "gemini", name: "Gemini", mono: false, words: ["gemini", "bard"]),
        Provider(key: "grok", name: "Grok", mono: true, words: ["grok", "xai"]),
        Provider(key: "hermes", name: "Hermes", mono: true, words: ["hermes"]),
        Provider(key: "perplexity", name: "Perplexity", mono: false, words: ["perplexity"]),
        Provider(key: "mistral", name: "Mistral", mono: false, words: ["mistral", "le chat"]),
        Provider(key: "deepseek", name: "DeepSeek", mono: false, words: ["deepseek"]),
        Provider(key: "copilot", name: "Copilot", mono: false, words: ["copilot"]),
        Provider(key: "cursor", name: "Cursor", mono: true, words: ["cursor"]),
        Provider(key: "qwen", name: "Qwen", mono: false, words: ["qwen"]),
        // Kimi's mark is a white K beside a blue dot, invisible on the light plate: drawn in the text colour instead.
        Provider(key: "kimi", name: "Kimi", mono: true, words: ["kimi", "moonshot"]),
        Provider(key: "meta", name: "Meta AI", mono: false, words: ["llama", "meta ai"]),
        Provider(key: "ollama", name: "Ollama", mono: true, words: ["ollama"]),
    ]

    /// The stored pick that means "draw a blobatar even if the name suggests a provider".
    static let blob = "blob"

    static func byKey(_ key: String?) -> Provider? { all.first { $0.key == key } }

    /// The provider a connection name suggests, if any ("My Claude" -> Claude).
    static func infer(_ label: String) -> Provider? {
        let lower = label.lowercased()
        return all.first { p in p.words.contains { lower.contains($0) } }
    }

    /// What a connection shows: the picked provider, the one its name suggests when nothing was picked, or nil for a
    /// blobatar.
    static func resolve(label: String, pick: String?) -> Provider? {
        byKey(pick) ?? (pick == nil ? infer(label) : nil)
    }
}

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

/// Blobatar figures as SwiftUI paths, built once per name.
enum BlobatarDrawing {
    struct Drawn {
        let path: Path
        let color: Color
    }

    private static let lock = NSLock()
    nonisolated(unsafe) private static var cache: [String: [Drawn]] = [:]

    static func marks(for seed: String) -> [Drawn] {
        let key = seed.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty ? "?" : seed
        if let hit = lock.withLock({ cache[key] }) { return hit }
        let drawn = Blobatar.figure(key).marks.map { mark -> Drawn in
            switch mark {
            case let .path(d, fill): Drawn(path: SVGPath.parse(d), color: color(fill))
            case let .circle(cx, cy, r, fill): Drawn(path: Path(ellipseIn: CGRect(x: cx - r, y: cy - r, width: 2 * r, height: 2 * r)), color: color(fill))
            }
        }
        lock.withLock {
            if cache.count > 256 { cache.removeAll() }
            cache[key] = drawn
        }
        return drawn
    }

    /// "#rrggbb".
    static func color(_ hex: String) -> Color {
        let value = UInt32(hex.dropFirst(), radix: 16) ?? 0
        return Color(uiColor: UIColor(hex: value))
    }
}

/// The subset of SVG path data the blobatar renderer writes: absolute M, L, H, V, C, Q and Z.
enum SVGPath {
    static func parse(_ d: String) -> Path {
        var path = Path()
        var command: Character = "M"
        var numbers: [CGFloat] = []
        var current = CGPoint.zero

        func flush() {
            var n = numbers[...]
            func take() -> CGFloat? { n.isEmpty ? nil : n.removeFirst() }
            func point() -> CGPoint? {
                guard let x = take(), let y = take() else { return nil }
                return CGPoint(x: x, y: y)
            }
            switch command {
            case "M":
                if let p = point() { path.move(to: p); current = p }
                while let p = point() { path.addLine(to: p); current = p }
            case "L":
                while let p = point() { path.addLine(to: p); current = p }
            case "H":
                while let x = take() { current = CGPoint(x: x, y: current.y); path.addLine(to: current) }
            case "V":
                while let y = take() { current = CGPoint(x: current.x, y: y); path.addLine(to: current) }
            case "C":
                while let c1 = point(), let c2 = point(), let p = point() { path.addCurve(to: p, control1: c1, control2: c2); current = p }
            case "Q":
                while let c = point(), let p = point() { path.addQuadCurve(to: p, control: c); current = p }
            case "Z":
                path.closeSubpath()
            default:
                break
            }
            numbers = []
        }

        var token = ""
        func endToken() {
            if !token.isEmpty, let v = Double(token) { numbers.append(CGFloat(v)) }
            token = ""
        }
        for ch in d {
            if ch.isLetter && ch != "e" {
                endToken()
                flush()
                command = ch
            } else if ch == " " || ch == "," {
                endToken()
            } else if ch == "-" && !token.isEmpty && token.last != "e" {
                endToken()
                token = "-"
            } else {
                token.append(ch)
            }
        }
        endToken()
        flush()
        return path
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
