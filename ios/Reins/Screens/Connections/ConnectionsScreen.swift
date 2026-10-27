import SwiftUI
import UIKit

/// Connections (the Connections board, 01): a standard tab root. Who can ask (the AI apps, then the computers) and what
/// they reach (the services with an account, and the MCP servers), each section with its own plus: an AI app and a
/// computer open "Connect", a service opens the list of every service.
struct ConnectionsScreen: View {
    @Environment(AppModel.self) private var model
    @Environment(\.feedback) private var feedback
    @State private var connecting = false
    @State private var addingService = false
    /// What the connect sheet chose; done once it has gone (a sheet cannot open while another is still closing).
    @State private var chosen: ConnectChoice?
    @State private var chosenService: String?
    @State private var showAllApps = false
    /// "Join as a second phone", when the server keeps the list from this phone.
    @State private var joining = false
    @State private var viewerJoin = ViewerJoinModel()

    /// AI apps: connections that did not pin a computer's key.
    private var apps: [ConnectionView] { model.connections.filter { $0.keyFingerprint == nil } }
    private var computers: [ConnectionView] { model.connections.filter { $0.keyFingerprint != nil } }
    private var reach: [ServiceView] { model.services.filter { !$0.accounts.isEmpty } }

    var body: some View {
        NextPage(bottom: 120) {
            StandardTitle("Connections")
                .accessibilityIdentifier("connectionsTitle")
            PlusSectionHeader(title: "AI apps", glyph: .globe, count: model.connectionsHidden ? nil : apps.count, addLabel: "Connect an AI app", addId: "connect") {
                connecting = true
            }
            .padding(.top, -4)
            RaisedGroup {
                // Not this phone's to read (it neither approves nor views): where they are, never "None yet".
                if model.connectionsHidden {
                    HiddenRow(id: "aisHidden") { joining = true }
                } else if apps.isEmpty {
                    EmptyRow(text: "None yet").accessibilityIdentifier("noConnections")
                }
                ForEach(showAllApps ? apps : Array(apps.prefix(4)), id: \.id) { ConnectionRow(connection: $0) }
                if apps.count > 4 && !showAllApps {
                    Button {
                        feedback.play(.expand(true))
                        withAnimation(.smooth(duration: 0.25)) { showAllApps = true }
                    } label: {
                        HStack(spacing: 12) {
                            Text("Show all").font(RFont.sans(15)).foregroundStyle(Palette.ink2).frame(maxWidth: .infinity, alignment: .leading)
                            Text("\(apps.count)").font(RFont.sans(15)).foregroundStyle(Palette.ink2)
                            RowChevron()
                        }
                        .padding(.horizontal, 14)
                        .frame(minHeight: 56)
                    }
                    .buttonStyle(RowPressStyle())
                    .accessibilityIdentifier("showAllApps")
                }
            }
            PlusSectionHeader(title: "Computers", glyph: .laptop, count: model.connectionsHidden ? nil : computers.count, addLabel: "Connect a computer", addId: "connectComputer") {
                connecting = true
            }
            RaisedGroup {
                if model.connectionsHidden {
                    HiddenRow(id: "computersHidden") { joining = true }
                } else if computers.isEmpty {
                    EmptyRow(text: "None yet").accessibilityIdentifier("noComputers")
                }
                ForEach(computers, id: \.id) { ConnectionRow(connection: $0) }
            }
            PlusSectionHeader(
                title: "Services", glyph: .link, count: reach.count + model.mcpServers.count, addLabel: "Add a service", addId: "addService"
            ) {
                addingService = true
            }
            RaisedGroup {
                if reach.isEmpty && model.mcpServers.isEmpty {
                    EmptyRow(text: "None yet").accessibilityIdentifier("noServices")
                }
                ForEach(reach, id: \.service) { ServiceRow(service: $0) }
                ForEach(model.mcpServers, id: \.id) { McpRow(server: $0) }
            }
        }
        .overlay(alignment: .bottom) {
            // The fade under the floating tab bar.
            LinearGradient(colors: [Palette.paper.opacity(0), Palette.paper], startPoint: .top, endPoint: .init(x: 0.5, y: 0.7))
                .frame(height: 110)
                .ignoresSafeArea()
                .allowsHitTesting(false)
        }
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
            #if DEBUG
            let args = ProcessInfo.processInfo.arguments
            if let i = args.firstIndex(of: "-show"), i + 1 < args.count {
                if args[i + 1] == "connectSheet" { connecting = true }
                if args[i + 1] == "addService" { addingService = true }
            }
            #endif
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
        .sheet(isPresented: $addingService, onDismiss: openService) {
            AddServiceSheet { id in
                chosenService = id
                addingService = false
            }
            .environment(model)
            .environment(\.feedback, feedback)
        }
        .presentationFeedback(addingService)
        .sheet(isPresented: $joining, onDismiss: { viewerJoin.cancel(model) }) {
            ViewerJoinSheet(vm: viewerJoin) { joining = false }
                .environment(model)
                .environment(\.feedback, feedback)
        }
        .presentationFeedback(joining)
    }

    private func openChosen() {
        guard let choice = chosen else { return }
        chosen = nil
        model.connectStartsTyped = choice == .typeCode
        model.openSheet(.connectComputer)
    }

    private func openService() {
        guard let id = chosenService else { return }
        chosenService = nil
        switch id {
        case "mcp": model.show(.mcpAdd)
        case "gmail": model.show(.gmail)
        default: model.show(.service(id))
        }
    }
}

/// What Connections says in words (VoiceOver's summary of the page).
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

    /// The tag beside an AI that has a mode of its own (Auto, Overnight, Lockdown, Bypass); none for one that follows
    /// every AI or is Manual on its own.
    static func ownMode(_ connectionId: String, _ settings: AutopilotSettings?) -> ModeKind? {
        guard let own = settings?.connections.first(where: { $0.connectionId == connectionId }) else { return nil }
        if own.bypassUntil != nil { return .bypass }
        if own.overnightUntil != nil { return .overnight }
        switch own.baseMode {
        case .auto?: return .auto
        case .lockdown?: return .lockdown
        case .overnight?: return .overnight
        default: return nil
        }
    }
}

/// A row in a raised group pressed: the row dims, as the boards' rows do.
struct RowPressStyle: ButtonStyle {
    func makeBody(configuration: Configuration) -> some View {
        configuration.label
            .background(configuration.isPressed ? Palette.paperSunken.opacity(0.6) : .clear)
            .contentShape(Rectangle())
    }
}

/// In place of a group's connections when the server keeps them from this phone: they are on the approving phone, and
/// Join asks it to add this one as a second phone (a viewer), which then sees them.
private struct HiddenRow: View {
    var id: String
    var onJoin: () -> Void

    var body: some View {
        HStack(spacing: 12) {
            Text("Visible on the approving phone")
                .font(RFont.sans(15))
                .foregroundStyle(Palette.ink2)
                .frame(maxWidth: .infinity, alignment: .leading)
                .accessibilityIdentifier(id)
            Button("Join", action: onJoin)
                .buttonStyle(SignalButtonLook(kind: .soft, size: .small))
                .fixedSize()
                .accessibilityLabel("Join as a second phone")
                .accessibilityIdentifier("join:\(id)")
        }
        .frame(minHeight: 56)
        .padding(.leading, 14)
        .padding(.trailing, 10)
    }
}

/// "None yet": a group with nothing in it.
private struct EmptyRow: View {
    var text: String

    var body: some View {
        Text(text)
            .font(RFont.sans(15))
            .foregroundStyle(Palette.ink2)
            .frame(maxWidth: .infinity, minHeight: 56, alignment: .leading)
            .padding(.horizontal, 14)
    }
}

/// An AI app or computer: its avatar, its name (cleaned, one line), the tag of a mode of its own, a chevron.
private struct ConnectionRow: View {
    var connection: ConnectionView
    @Environment(AppModel.self) private var model
    @Environment(\.feedback) private var feedback

    var body: some View {
        let mode = ConnectionsText.ownMode(connection.id, model.autopilot)
        Button {
            model.show(.connection(connection.id))
            feedback.defaultTap()
        } label: {
            TileRow(untrustedName(connection.label, limit: 40)) {
                MarkPlate(size: 32) { ConnectionAvatar(connection: connection, size: 32) }
            } trailing: {
                if let mode { Pill(mode.name, tone: mode == .auto ? .violet : mode == .overnight ? .night : mode == .bypass ? .danger : .warning, mode: mode) }
            }
        }
        .buttonStyle(RowPressStyle())
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(untrustedName(connection.label, limit: 40) + (mode.map { ", \($0.name)" } ?? ""))
        .accessibilityAddTraits(.isButton)
        .accessibilityIdentifier("connection:\(connection.id)")
    }
}

/// A service with an account: its logo, its name, the account (or how many), a chevron.
private struct ServiceRow: View {
    var service: ServiceView
    @Environment(AppModel.self) private var model
    @Environment(\.feedback) private var feedback

    var body: some View {
        Button {
            model.show(service.service == "gmail" ? .gmail : .service(service.service))
            feedback.defaultTap()
        } label: {
            TileRow(service.name) {
                MarkPlate(size: 32) { ServiceAvatar(service: service.service, size: 32) }
            } trailing: {
                if service.accounts.count == 1, let only = service.accounts.first {
                    Text(ServiceCopy.accountTitle(service, only.account))
                        .font(RFont.sans(15))
                        .foregroundStyle(Palette.ink2)
                        .lineLimit(1)
                        .truncationMode(.middle)
                        .environment(\.layoutDirection, .leftToRight)
                } else {
                    HStack(spacing: 4) {
                        GlyphIcon(.person, size: 14)
                        Text("\(service.accounts.count)")
                    }
                    .font(RFont.sans(15))
                    .foregroundStyle(Palette.ink2)
                }
            }
        }
        .buttonStyle(RowPressStyle())
        .accessibilityElement(children: .ignore)
        .accessibilityLabel("\(service.name), \(ServiceCopy.summary(service))")
        .accessibilityAddTraits(.isButton)
        .accessibilityIdentifier("service:\(service.service)")
    }
}

/// An MCP server the user added: the plug, its name (theirs, cleaned), its status when it is not working.
private struct McpRow: View {
    var server: McpServerView
    @Environment(AppModel.self) private var model
    @Environment(\.feedback) private var feedback

    var body: some View {
        Button {
            model.show(.mcpServer(server.id))
            feedback.defaultTap()
        } label: {
            TileRow(untrustedName(server.name, limit: 40)) {
                MarkPlate(size: 32) { ServiceAvatar(service: "mcp", size: 32) }
            } trailing: {
                if server.status == "ok" {
                    Text(McpLogic.toolCount(server.tools.count)).font(RFont.sans(15)).foregroundStyle(Palette.ink2).lineLimit(1)
                } else {
                    McpStatusPill(status: server.status)
                }
            }
        }
        .buttonStyle(RowPressStyle())
        .accessibilityElement(children: .combine)
        .accessibilityAddTraits(.isButton)
        .accessibilityIdentifier("mcp:\(server.id)")
    }
}

private enum ConnectChoice { case scan, typeCode }

/// "Connect" (the Connect board, 01): an AI app by the address to paste into it (copied here), or a computer by the
/// code it shows, scanned or typed.
private struct ConnectSheet: View {
    var onChoose: (ConnectChoice) -> Void
    @Environment(AppModel.self) private var model
    @Environment(\.feedback) private var feedback
    @Environment(\.dismiss) private var dismiss
    @State private var copied = false

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            SheetHeading("Connect") { dismiss() }
            SectionHeader("AI app", glyph: .link)
            RaisedGroup {
                HStack(spacing: 8) {
                    Text(verbatim: SettingsText.host(model.mcpAddress))
                        .font(RFont.mono(14.5))
                        .foregroundStyle(Palette.ink)
                        .lineLimit(1)
                        .truncationMode(.middle)
                        .frame(maxWidth: .infinity, alignment: .leading)
                        .environment(\.layoutDirection, .leftToRight)
                        .accessibilityIdentifier("mcpUrl")
                    RoundHeaderButton(glyph: copied ? .check : .copy, label: "Copy \(model.mcpAddress)", action: copy)
                        .accessibilityIdentifier("copyMcpUrl")
                }
                .padding(.leading, 14)
                .padding(.trailing, 12)
                .frame(minHeight: 56)
            }
            .accessibilityElement(children: .contain)
            .accessibilityIdentifier("connectAi")
            SectionHeader("Computer", glyph: .laptop)
            RaisedGroup {
                HStack(spacing: 10) {
                    Button {
                        feedback.play(.tap)
                        onChoose(.scan)
                    } label: {
                        Label { Text("Scan") } icon: { GlyphIcon(.qrCode, size: 18) }
                    }
                    .buttonStyle(SignalButtonStyle(.inverse, size: .small))
                    .accessibilityIdentifier("sheetScanQr")
                    Button {
                        feedback.play(.tap)
                        onChoose(.typeCode)
                    } label: {
                        Label { Text("Type") } icon: { GlyphIcon(.pencil, size: 18) }
                    }
                    .buttonStyle(SignalButtonStyle(.soft, size: .small))
                    .accessibilityIdentifier("sheetTypeCode")
                }
                .padding(.horizontal, 14)
                .padding(.vertical, 12)
            }
            .accessibilityElement(children: .contain)
            .accessibilityIdentifier("connectComputerCard")
            Spacer(minLength: 0)
        }
        .padding(.top, 14)
        .frame(maxHeight: .infinity, alignment: .top)
        .presentationDetents([.height(372), .large])
        .presentationDragIndicator(.visible)
        .presentationCornerRadius(32)
        .presentationBackground(Palette.paper)
        .accessibilityIdentifier("connectSheet")
    }

    private func copy() {
        copyPlain(model.mcpAddress, feedback: feedback)
        withAnimation(.smooth(duration: 0.2)) { copied = true }
        Task {
            try? await Task.sleep(for: .seconds(2))
            withAnimation(.smooth(duration: 0.2)) { copied = false }
        }
    }
}

/// A sheet's title row (the kit's `sheet(…, title)`): display 22 and the round close button.
struct SheetHeading: View {
    var title: String
    var onClose: () -> Void

    init(_ title: String, onClose: @escaping () -> Void) {
        self.title = title
        self.onClose = onClose
    }

    var body: some View {
        HStack {
            Text(title)
                .textRole(.cardAsk)
                .foregroundStyle(Palette.ink)
                .accessibilityAddTraits(.isHeader)
                .frame(maxWidth: .infinity, alignment: .leading)
            RoundHeaderButton(glyph: .close, label: "Close", action: onClose)
                .accessibilityIdentifier("closeSheet")
        }
        .padding(.leading, 20)
        .padding(.trailing, 20)
        .padding(.top, 6)
        .padding(.bottom, 4)
    }
}

/// "Add a service": every service Reins can connect (those not available say so), and an MCP server by its address.
private struct AddServiceSheet: View {
    var onChoose: (String) -> Void
    @Environment(AppModel.self) private var model
    @Environment(\.dismiss) private var dismiss

    private var services: [ServiceView] { model.services + [ServiceCopy.sms] }

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 0) {
                SheetHeading("Add a service") { dismiss() }
                RaisedGroup {
                    ForEach(services, id: \.service) { service in
                        Button { onChoose(service.service) } label: {
                            TileRow(service.name, chevron: service.available) {
                                MarkPlate(size: 32) { ServiceAvatar(service: service.service, size: 32) }
                            } trailing: {
                                let summary = ServiceCopy.summary(service)
                                if !summary.isEmpty { Text(summary).font(RFont.sans(15)).foregroundStyle(Palette.ink2).lineLimit(1) }
                            }
                        }
                        .buttonStyle(RowPressStyle())
                        .disabled(!service.available && service.service == ServiceCopy.sms.service)
                        .accessibilityIdentifier("addService:\(service.service)")
                    }
                    Button { onChoose("mcp") } label: {
                        TileRow("MCP server") {
                            MarkPlate(size: 32) { ServiceAvatar(service: "mcp", size: 32) }
                        } trailing: { EmptyView() }
                    }
                    .buttonStyle(RowPressStyle())
                    .accessibilityIdentifier("addMcp")
                }
                .padding(.top, 12)
            }
            .padding(.top, 14)
            .padding(.bottom, 24)
        }
        .presentationDetents([.large])
        .presentationDragIndicator(.visible)
        .presentationCornerRadius(32)
        .presentationBackground(Palette.paper)
        .accessibilityIdentifier("addServiceSheet")
    }
}
