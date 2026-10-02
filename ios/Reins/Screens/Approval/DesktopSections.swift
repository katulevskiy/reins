import SwiftUI

// Requests shown in their own terms (the Android app's NewSections.kt): a call to an MCP server's tool, a file the
// change uses, and the desktop app's question, secrets and SSH sign-in.

/// Which server, which tool, what it says it does, the exact arguments, and whether it changes things.
struct McpCallSection: View {
    var call: McpCallView

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            Card(padding: 16) {
                VStack(alignment: .leading, spacing: 6) {
                    HStack(spacing: 12) {
                        ServiceAvatar(service: "mcp", size: 36)
                        VStack(alignment: .leading, spacing: 1) {
                            Text(untrusted(call.serverName)).font(RFont.sans(15.5, .semibold)).foregroundStyle(Palette.text).lineLimit(1)
                            Text(mcpHostLabel(call.serverUrl))
                                .font(RFont.mono(12.5))
                                .foregroundStyle(Palette.tertiary)
                                .lineLimit(1)
                                .environment(\.layoutDirection, .leftToRight)
                        }
                        .frame(maxWidth: .infinity, alignment: .leading)
                        TintTag(text: call.readOnly ? "Read only" : "Changes things", tint: call.readOnly ? Palette.success : Palette.send)
                            .accessibilityIdentifier("mcpEffect")
                    }
                    Hairline(inset: 0).padding(.vertical, 6)
                    let title = untrusted(call.title)
                    Text(title.isEmpty ? untrusted(call.tool) : title)
                        .font(RFont.sans(17, .semibold))
                        .foregroundStyle(Palette.text)
                        .accessibilityIdentifier("mcpTool")
                    if call.title != call.tool {
                        Text(untrusted(call.tool)).font(RFont.mono(12.5)).foregroundStyle(Palette.tertiary).lineLimit(1)
                    }
                    if !call.description.trimmingCharacters(in: .whitespaces).isEmpty {
                        Text(untrusted(call.description)).font(RFont.sans(14)).foregroundStyle(Palette.secondary).lineLimit(6)
                    }
                    Caption("Arguments").padding(.top, 8)
                    let args = untrusted(call.argumentsJson)
                    MonoBox(text: args.isEmpty ? "{}" : args).accessibilityIdentifier("mcpArguments")
                    Text("The server describes its own tools. Read the arguments: they are exactly what is sent.")
                        .font(RFont.sans(12.5))
                        .foregroundStyle(Palette.tertiary)
                        .padding(.top, 4)
                }
            }
            if call.destructive {
                Banner("The server marks this tool as destructive. It is asked for every time and can never be allowed in advance.", kind: .warning)
                    .accessibilityIdentifier("mcpDestructive")
            }
        }
        .padding(.horizontal, 16)
        .padding(.vertical, 8)
        .accessibilityContainer("mcpCall")
    }
}

/// The file an AI uploaded for this change, as the server saw it.
struct AttachedFileSection: View {
    var blob: BlobView

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            SectionHeader("Attached file")
            FileCard(blob: blob)
        }
        .padding(.horizontal, 16)
        .padding(.vertical, 8)
        .accessibilityContainer("attachedFile")
    }
}

/// A yes-or-no question: the question large, what would happen exactly, and what a standing answer would cover.
struct AskSection: View {
    var ask: AskView

    var body: some View {
        Card(padding: 18) {
            VStack(alignment: .leading, spacing: 10) {
                Text(untrusted(ask.question))
                    .font(RFont.sans(22, .semibold))
                    .foregroundStyle(Palette.text)
                    .accessibilityIdentifier("askQuestion")
                if let detail = ask.detail.map(untrusted), !detail.isEmpty {
                    MonoBox(text: detail, lineLimit: 30).accessibilityIdentifier("askDetail")
                }
                if let topic = ask.topic.map(untrusted), !topic.isEmpty {
                    HStack(spacing: 8) {
                        Caption("About")
                        TintTag(text: topic, tint: nil, mono: true).accessibilityIdentifier("askTopic")
                    }
                }
            }
        }
        .padding(.horizontal, 16)
        .padding(.vertical, 8)
        .accessibilityContainer("ask")
    }
}

/// Secrets the desktop app wants for one command: names and fields only, never the values, and for how long.
struct SecretsSection: View {
    var secrets: SecretReleaseView

    var body: some View {
        Card(padding: 16) {
            VStack(alignment: .leading, spacing: 6) {
                Caption("For")
                MonoBox(text: untrusted(secrets.command), lineLimit: 6).accessibilityIdentifier("secretsCommand")
                if let purpose = secrets.purpose.map(untrusted), !purpose.isEmpty {
                    Text("“\(purpose)”").font(RFont.sans(14.5)).foregroundStyle(Palette.secondary).padding(.top, 2)
                }
                Caption(secrets.items.count == 1 ? "Secret" : "Secrets (\(secrets.items.count))").padding(.top, 8)
                ForEach(Array(secrets.items.enumerated()), id: \.offset) { i, item in
                    HStack(spacing: 10) {
                        Image(systemName: "key.fill").font(.system(size: 13, weight: .semibold)).foregroundStyle(Palette.accent)
                        Text(untrusted(item)).font(RFont.sans(15.5, .medium)).foregroundStyle(Palette.text).lineLimit(2)
                    }
                    .accessibilityIdentifier("secret:\(i)")
                }
                Text("The desktop app may keep them \(leaseLabel(secrets.leaseSecs)), in memory only. Their values are never shown here or sent anywhere else.")
                    .font(RFont.sans(13.5))
                    .foregroundStyle(Palette.secondary)
                    .padding(.top, 8)
                    .accessibilityIdentifier("secretsLease")
            }
        }
        .padding(.horizontal, 16)
        .padding(.vertical, 8)
        .accessibilityContainer("secrets")
    }
}

/// An SSH sign-in signed on this phone: which server, with which key.
struct SshSection: View {
    var ssh: SshSignView

    var body: some View {
        Card(padding: 16) {
            VStack(alignment: .leading, spacing: 6) {
                Text("Sign in to \(untrusted(sshTarget(host: ssh.host, hostKey: ssh.hostKey))) with \(untrusted(ssh.keyName))")
                    .font(RFont.sans(18, .semibold))
                    .foregroundStyle(Palette.text)
                    .accessibilityIdentifier("sshWhat")
                Caption("Key").padding(.top, 8)
                MonoBox(text: untrusted(ssh.keyFingerprint)).accessibilityIdentifier("sshKey")
                if let hostKey = ssh.hostKey.map(untrusted), !hostKey.isEmpty {
                    Caption("Server's host key").padding(.top, 4)
                    MonoBox(text: hostKey).accessibilityIdentifier("sshHostKey")
                }
                Text("The private key stays on this phone; it signs this one sign-in.")
                    .font(RFont.sans(13))
                    .foregroundStyle(Palette.tertiary)
                    .padding(.top, 4)
            }
        }
        .padding(.horizontal, 16)
        .padding(.vertical, 8)
        .accessibilityContainer("ssh")
    }
}
