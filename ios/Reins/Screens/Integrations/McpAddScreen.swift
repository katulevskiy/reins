import SwiftUI

/// A server by its address; a name and an access token are optional.
struct McpAddScreen: View {
    @Environment(AppModel.self) private var model
    @State private var mcp: McpModel?
    @State private var url = ""
    @State private var name = ""
    @State private var token = ""
    @FocusState private var focus: Field?

    private enum Field { case url, name, token }

    var body: some View {
        List {
            Section {
                VStack(alignment: .leading, spacing: 12) {
                    InputWell {
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
                    InputWell {
                        TextField("Name (optional)", text: $name)
                            .font(RFont.sans(16))
                            .focused($focus, equals: .name)
                            .submitLabel(.next)
                            .onSubmit { focus = .token }
                            .accessibilityIdentifier("mcpName")
                    }
                    InputWell {
                        SecureField("Access token (optional)", text: $token)
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
                    GroupFooter(
                        "Paste the address the service gives for its MCP server. Reins connects from this phone; if the server wants you to sign in, its page opens here.\n\n" +
                            "The token is only for servers that give you one instead of a sign-in. It is kept encrypted on this phone."
                    )
                    ActionButton(title: "Add", busy: busy, enabled: !url.trimmingCharacters(in: .whitespaces).isEmpty, action: submit)
                        .padding(.top, 6)
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
                .disabled(busy)
                .plainListRow(top: 8, bottom: 28)
            }
        }
        .integrationList()
        .navigationTitle("Add MCP server")
        .navigationSubtitle("Integrations")
        .navigationBarTitleDisplayMode(.large)
        .animation(.smooth, value: mcp?.error)
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
        Task {
            guard let id = await mcp.add(url: url, name: name, token: sent) else { return }
            // From the add page to the new server's page, unless the user went somewhere else meanwhile.
            if model.path(model.section).last == .mcpAdd {
                model.back()
                model.push(.mcpServer(id))
            }
        }
    }
}
