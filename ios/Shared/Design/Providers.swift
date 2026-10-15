import Foundation

/// An AI provider the user can pick as a connection's icon. Its logo is `provider-<key>` in Brand.xcassets (see
/// BRAND-NOTICE.md); `mono` logos are single-colour and take the text colour, the others keep their brand colours.
/// In Shared so widgets and Live Activities show the same logos as the app.
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

/// A computer's platform, which it shows as its icon: its logo is `platform-<key>` in Brand.xcassets (Simple Icons,
/// see BRAND-NOTICE.md). Apple's and Linux's marks take the text colour, Windows' its brand blue (`brand`).
struct Platform: Identifiable, Equatable {
    let key: String
    let name: String
    let brand: UInt32?

    var id: String { key }
    var asset: String { "platform-\(key)" }
}

enum Platforms {
    static let all: [Platform] = [
        Platform(key: "macos", name: "macOS", brand: nil),
        Platform(key: "windows", name: "Windows", brand: 0x0078D4),
        Platform(key: "linux", name: "Linux", brand: nil),
    ]

    static func byKey(_ key: String?) -> Platform? { all.first { $0.key == key } }
}

/// What a connection's avatar draws: a provider's logo, a platform's logo, or a blobatar (the seed's figure, with the
/// shape and colour the connection keeps, when it keeps them).
enum AvatarLook: Equatable {
    case provider(Provider)
    case platform(Platform)
    case blob(seed: String, shape: Int?, colour: Int?)

    /// For `icon`, a provider's or a platform's key or "blob" (an unknown one draws the blobatar too), or nil, the
    /// older rule: the provider `label` suggests, else the blobatar. The blobatar is `blobatar`, else seeded by `label`.
    static func of(label: String, icon: String?, blobatar: BlobatarLook? = nil) -> AvatarLook {
        if let platform = Platforms.byKey(icon) { return .platform(platform) }
        if let provider = Providers.resolve(label: label, pick: icon) { return .provider(provider) }
        return .blob(seed: blobatar?.seed ?? label, shape: blobatar?.shape, colour: blobatar?.colour)
    }

    var isBlob: Bool {
        if case .blob = self { return true }
        return false
    }
}
