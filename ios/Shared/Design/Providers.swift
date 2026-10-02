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
