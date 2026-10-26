import SwiftUI

/// A server by its address; a name and an access token are optional.
struct McpAddScreen: View {
    @Environment(AppModel.self) private var model
    @State private var mcp: McpModel?
    @State private var url = ""
    @State private var name = ""
    @State private var token = ""
    @State private var active = false
    @FocusState private var focus: Field?

    private enum Field { case url, name, token }

    var body: some View {
        NextPage {
            QuietTitle(title: "Add MCP server") {
                MarkPlate(size: 36) { ServiceAvatar(service: "mcp", size: 36) }
            }
            SectionHeader("Server", quiet: true)
            FlatFields {
                InputRow(glyph: .globe) {
                    TextField("https://mcp.example.com/mcp", text: $url)
                        .font(RFont.mono(15))
                        .keyboardType(.URL)
                        .textContentType(.URL)
                        .textInputAutocapitalization(.never)
                        .autocorrectionDisabled()
                        .focused($focus, equals: .url)
                        .submitLabel(.next)
                        .onSubmit { focus = .name }
                        .accessibilityIdentifier("mcpUrl")
                } trailing: {
                    PasteInto(label: "Paste the address") { url = $0 }
                }
                InputRow(glyph: .pencil) {
                    TextField("Name", text: $name)
                        .focused($focus, equals: .name)
                        .submitLabel(.next)
                        .onSubmit { focus = .token }
                        .accessibilityIdentifier("mcpName")
                }
                InputRow(glyph: .vault) {
                    SecureField("Token", text: $token)
                        .font(RFont.mono(15))
                        .textInputAutocapitalization(.never)
                        .autocorrectionDisabled()
                        .focused($focus, equals: .token)
                        .submitLabel(.go)
                        .onSubmit(submit)
                        .accessibilityIdentifier("mcpToken")
                } trailing: {
                    PasteInto(label: "Paste the token") { pasted in
                        token = pasted
                        clearClipboard()
                    }
                }
            }
            GroupFooter(
                "Paste the address the service gives for its MCP server. The name and the token are optional. Reins connects from this phone; if the server wants you to sign in, its page opens here.\n\n" +
                    "The token is only for servers that give you one instead of a sign-in. It is kept encrypted on this phone."
            )
            .padding(.horizontal, 20)
            VStack(spacing: 10) {
                Button(action: submit) { BusyLabel(title: "Add", glyph: nil, busy: busy) }
                    .buttonStyle(SignalButtonStyle(.inverse))
                    .disabled(busy || url.trimmingCharacters(in: .whitespaces).isEmpty)
                    .accessibilityIdentifier("mcpAddSubmit")
                if mcp?.signingIn != nil {
                    IntegrationBanner(text: "Finish signing in on the page that opened.")
                        .accessibilityIdentifier("mcpSigningIn")
                }
                if let error = mcp?.error {
                    IntegrationBanner(text: untrusted(error), kind: .error)
                        .accessibilityIdentifier("mcpError")
                }
            }
            .padding(.horizontal, 20)
            .padding(.top, 8)
        }
        .disabled(busy)
        .quietPageBar("Add MCP server")
        .animation(.smooth, value: mcp?.error)
        .onAppear { active = true }
        .onDisappear { active = false }
        .task {
            if mcp == nil { mcp = McpModel(model: model) }
            mcp?.error = nil
            if url.isEmpty { focus = .url }
        }
    }

    private var busy: Bool { mcp?.busy ?? false }

    private func submit() {
        guard let mcp, !busy, !url.trimmingCharacters(in: .whitespaces).isEmpty else { return }
        let sent = token
        token = ""
        focus = nil
        let section = model.section
        let path = model.path(section)
        Task {
            guard let id = await mcp.add(url: url, name: name, token: sent) else { return }
            // From the add page to the new server's page, unless the user went somewhere else meanwhile.
            if active, model.section == section, model.path(section) == path, path.last == .mcpAdd {
                model.back()
                model.push(.mcpServer(id))
            }
        }
    }
}
