#if DEBUG
import SwiftUI

/// Launched with `-gallery`: every avatar on one page, to check logos, tints and blobatars in light and dark.
struct AvatarGallery: View {
    static var requested: Bool { ProcessInfo.processInfo.arguments.contains("-gallery") }

    private static let services = ["gmail", "telegram", "github", "gitlab", "codeberg", "bitbucket", "gcalendar", "gcontacts",
                                   "device_calendar", "device_contacts", "sms", "vault", "mcp:demo", "unknown"]
    private static let names = ["alain", "Claude Desktop", "my laptop", "Team Rocket 3", "user-0", "Build bot", "Ana", "Kitchen iPad",
                                "Research agent", "zed", "x", "office-mac", "night shift", "Codex CLI?", "pixel", "Long running worker",
                                "Ollama box", "dev@example.com", "Hermes", "Mallory"]

    @State private var pick: String?

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 18) {
                SectionHeader("Providers")
                grid(Providers.all.map { ($0.name, AnyView(ProviderAvatar(provider: $0, size: 48))) })
                SectionHeader("Services")
                grid(Self.services.map { ($0, AnyView(ServiceAvatar(service: $0, size: 48))) })
                SectionHeader("Blobatars")
                grid(Self.names.map { ($0, AnyView(BlobAvatar(seed: $0, size: 48))) })
                SectionHeader("Connection avatars (name inference)")
                grid(["My Claude", "chatgpt work", "Le Chat", "llama server", "random thing"].map {
                    ($0, AnyView(ConnectionAvatar(label: $0, pick: nil, size: 48)))
                })
                SectionHeader("Provider picker")
                Card {
                    VStack(alignment: .leading, spacing: 10) {
                        ProviderPicker(label: "Claude Desktop", selection: pick) { pick = $0 }
                        Text(ProviderPicker.footnote).font(RFont.sans(12.5)).foregroundStyle(Palette.tertiary)
                    }
                }
            }
            .padding(16)
        }
        .background(Palette.background.ignoresSafeArea())
    }

    private func grid(_ items: [(String, AnyView)]) -> some View {
        LazyVGrid(columns: [GridItem(.adaptive(minimum: 72), spacing: 10)], spacing: 12) {
            ForEach(items.indices, id: \.self) { i in
                VStack(spacing: 4) {
                    items[i].1
                    Text(items[i].0).font(RFont.sans(11)).foregroundStyle(Palette.secondary).lineLimit(1)
                }
            }
        }
    }
}
#endif
