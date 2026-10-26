import SwiftUI
import UIKit

/// Connections: both ends of the wire (iOS-Connections.png). Who can ask (the AI apps and the computers) on top, what
/// they can reach (the services and the MCP servers) below, in one list; the plus opens "Connect something". It is
/// Settings' Connected group and the Integrations page put together, their views moved here as they were.
struct ConnectionsScreen: View {
    @Environment(AppModel.self) private var model
    @Environment(\.feedback) private var feedback
    @State private var connecting = false
    /// What the connect sheet chose; done once it has gone (a sheet cannot open while another is still closing).
    @State private var chosen: ConnectChoice?

    var body: some View {
        List {
            Section {
                VStack(alignment: .leading, spacing: 6) {
                    PageHeader(title: "Connections") {
                        RoundPlusButton(label: "Connect an AI app or a computer") {
                            connecting = true
                        }
                        .accessibilityIdentifier("connect")
                    }
                    Text(ConnectionsText.mood(model.connections))
                        .textRole(.body)
                        .foregroundStyle(Palette.ink2)
                        .accessibilityIdentifier("connectionsMood")
                }
                .listRowBackground(Color.clear)
                .listRowInsets(EdgeInsets(top: 0, leading: 4, bottom: 0, trailing: 4))
            }
            WhoCanAskSection()
            ServicesSection(title: "What they can reach")
            McpServersSection(servers: model.mcpServers)
        }
        .reinsGrouped()
        .navigationTitle("Connections")
        .rootNavigationBar()
        .refreshable {
            feedback.play(.refresh)
            await model.refreshConnections()
            await model.refreshPending()
        }
        .task {
            await model.refreshConnections()
            await model.refreshPending()
            DemoIntegrationsLaunch.openOnce(model)
        }
        .sheet(isPresented: $connecting, onDismiss: openChosen) {
            ConnectSheet { choice in
                chosen = choice
                connecting = false
            }
            .environment(model)
            .environment(\.feedback, feedback)
        }
        .presentationFeedback(connecting)
    }

    private func openChosen() {
        guard let choice = chosen else { return }
        chosen = nil
        model.connectStartsTyped = choice == .typeCode
        model.openSheet(.connectComputer)
    }
}

/// What Connections says in words.
enum ConnectionsText {
    /// "Three AIs and a computer can ask.", "Nothing can ask yet."
    static func mood(_ connections: [ConnectionView]) -> String {
        let computers = connections.filter { $0.keyFingerprint != nil }.count
        return mood(ais: connections.count - computers, computers: computers)
    }

    static func mood(ais: Int, computers: Int) -> String {
        let parts = [count(ais, one: "one AI", many: "AIs"), count(computers, one: "a computer", many: "computers")].compactMap { $0 }
        guard !parts.isEmpty else { return "Nothing can ask yet." }
        let sentence = parts.joined(separator: " and ") + " can ask."
        return sentence.prefix(1).uppercased() + sentence.dropFirst()
    }

    private static let words = ["zero", "one", "two", "three", "four", "five", "six", "seven", "eight", "nine", "ten"]

    private static func count(_ n: Int, one: String, many: String) -> String? {
        switch n {
        case ...0: nil
        case 1: one
        default: (n < words.count ? words[n] : "\(n)") + " " + many
        }
    }
}

/// Who can ask: the AI apps, then the computers, each opening its page.
private struct WhoCanAskSection: View {
    @Environment(AppModel.self) private var model
    @Environment(\.feedback) private var feedback

    /// AI apps first, then the computers, which pinned their key when they paired.
    private var connections: [ConnectionView] {
        model.connections.filter { $0.keyFingerprint == nil } + model.connections.filter { $0.keyFingerprint != nil }
    }

    var body: some View {
        Section {
            if connections.isEmpty {
                NoAccountsRow(title: "Nothing connected yet", message: "Connect an AI app or a computer with the plus above.")
                    .accessibilityIdentifier("noConnections")
                    .cardRow()
            }
            ForEach(connections, id: \.id) { connectionRow($0) }
        } header: {
            GroupHeader("Who can ask")
        }
    }

    private func connectionRow(_ connection: ConnectionView) -> some View {
        Button {
            model.show(.connection(connection.id))
            feedback.defaultTap()
        } label: {
            SettingsRowLabel(title: untrusted(connection.label), subtitle: SettingsText.connectionLine(connection)) {
                ConnectionAvatar(connection: connection, size: 32)
            } trailing: {
                EmptyView()
            }
        }
        .buttonStyle(.plain)
        .accessibilityLabel("\(untrusted(connection.label)), \(SettingsText.connectionLine(connection))")
        .accessibilityIdentifier("connection:\(connection.id)")
        .cardRow()
    }
}

private enum ConnectChoice { case scan, typeCode }

/// "+ Connect": an AI app by the address to paste (copied here), or a computer by the code it shows.
private struct ConnectSheet: View {
    var onChoose: (ConnectChoice) -> Void
    @Environment(AppModel.self) private var model
    @Environment(\.feedback) private var feedback
    @Environment(\.dismiss) private var dismiss
    @State private var help = false
    @State private var copied = false

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            HStack {
                Text("Connect")
                    .font(RFont.sans(18, .semibold))
                    .foregroundStyle(Palette.ink)
                    .accessibilityAddTraits(.isHeader)
                Spacer()
                GlassIconButton(symbol: "xmark", size: 34, label: "Close") { dismiss() }
                    .accessibilityIdentifier("closeSheet")
            }
            .padding(.bottom, 4)
            aiApp
            computer
        }
        .padding(.horizontal, 16)
        .padding(.top, 20)
        .frame(maxHeight: .infinity, alignment: .top)
        .presentationDetents([.height(help ? 470 : 400), .large])
        .presentationDragIndicator(.visible)
        .presentationBackground(Palette.paper)
        .accessibilityIdentifier("connectSheet")
    }

    private var aiApp: some View {
        VStack(alignment: .leading, spacing: 0) {
            HStack(spacing: 12) {
                HStack(spacing: -10) {
                    ConnectionAvatar(label: "Claude", pick: "claude", size: 34)
                        .overlay(Circle().strokeBorder(Palette.paperRaised, lineWidth: 2.5))
                        .zIndex(1)
                    ConnectionAvatar(label: "ChatGPT", pick: "openai", size: 34)
                }
                .accessibilityHidden(true)
                VStack(alignment: .leading, spacing: 1) {
                    Text("AI app").font(RFont.sans(16.5, .semibold)).foregroundStyle(Palette.ink)
                    Text("Claude, ChatGPT and others").font(RFont.sans(13)).foregroundStyle(Palette.inkMeta)
                }
                .frame(maxWidth: .infinity, alignment: .leading)
                Button {
                    let next = !help
                    withAnimation(.smooth(duration: 0.25)) { help = next }
                    feedback.play(.expand(next))
                } label: {
                    Image(systemName: "questionmark.circle")
                        .font(.system(size: 17))
                        .foregroundStyle(help ? Palette.violet : Palette.inkMeta)
                        .frame(width: 36, height: 36)
                        .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .accessibilityLabel(help ? "Hide help" : "Help")
                .accessibilityIdentifier("help")
            }
            if help {
                Text("In Claude.ai, ChatGPT or another AI app, add a custom connector (MCP) and paste this address. It then asks you here before it reads or sends anything.")
                    .font(RFont.sans(13))
                    .foregroundStyle(Palette.ink2)
                    .fixedSize(horizontal: false, vertical: true)
                    .padding(.top, 10)
                    .transition(.opacity)
            }
            Button(action: copy) {
                HStack(spacing: 10) {
                    Text(SettingsText.host(model.mcpAddress))
                        .font(RFont.mono(13))
                        .foregroundStyle(Palette.ink)
                        .lineLimit(1)
                        .truncationMode(.middle)
                        .frame(maxWidth: .infinity, alignment: .leading)
                    Label(copied ? "Copied" : "Copy", systemImage: copied ? "checkmark" : "doc.on.doc")
                        .font(RFont.sans(14, .semibold))
                        .foregroundStyle(Palette.violet)
                }
                .environment(\.layoutDirection, .leftToRight)
                .padding(.leading, 14)
                .padding(.trailing, 12)
                .padding(.vertical, 11)
                .background(Palette.paper, in: RoundedRectangle(cornerRadius: 13, style: .continuous))
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .padding(.top, 14)
            .accessibilityLabel("Copy \(model.mcpAddress)")
            .accessibilityIdentifier("copyMcpUrl")
        }
        .padding(16)
        .background(Palette.paperRaised, in: RoundedRectangle(cornerRadius: 22, style: .continuous))
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("connectAi")
    }

    private var computer: some View {
        VStack(alignment: .leading, spacing: 14) {
            HStack(spacing: 12) {
                SettingsTile(symbol: "laptopcomputer", tint: Palette.kindPair)
                VStack(alignment: .leading, spacing: 1) {
                    Text("Computer").font(RFont.sans(16.5, .semibold)).foregroundStyle(Palette.ink)
                    // A key, not a String, so the command shows in code type.
                    Text(LocalizedStringKey("Desktop app or `reins login`")).font(RFont.sans(13)).foregroundStyle(Palette.inkMeta)
                }
            }
            HStack(spacing: 10) {
                Button {
                    feedback.play(.tap)
                    onChoose(.scan)
                } label: {
                    Label("Scan QR code", systemImage: "qrcode.viewfinder")
                }
                .buttonStyle(CapsuleButtonStyle(kind: .primary, height: 44))
                .accessibilityIdentifier("sheetScanQr")
                Button {
                    feedback.play(.tap)
                    onChoose(.typeCode)
                } label: {
                    Text("Type code").font(RFont.sans(15, .semibold))
                }
                .buttonStyle(CapsuleButtonStyle(kind: .secondary, height: 44))
                .frame(width: 124)
                .accessibilityIdentifier("sheetTypeCode")
            }
        }
        .padding(16)
        .background(Palette.paperRaised, in: RoundedRectangle(cornerRadius: 22, style: .continuous))
        .accessibilityElement(children: .contain)
        .accessibilityIdentifier("connectComputerCard")
    }

    private func copy() {
        UIPasteboard.general.string = model.mcpAddress
        feedback.play(.copied)
        withAnimation(.smooth(duration: 0.2)) { copied = true }
        Task {
            try? await Task.sleep(for: .seconds(2))
            withAnimation(.smooth(duration: 0.2)) { copied = false }
        }
    }
}

