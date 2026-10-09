import SwiftUI

/// Settings > Devices (the Android app's `DevicesViewModel`): the account's devices, and signing one out (a lost
/// phone).
@Observable
@MainActor
final class DevicesModel {
    /// Nil until first read.
    private(set) var devices: [DeviceView]?
    /// The eight digits of this phone's key, which `reins vault add` asks for once.
    private(set) var phoneKey: String?
    private(set) var busy = false
    var error: String?
    var message: String?

    func load(_ model: AppModel) async {
        do {
            devices = try await model.core.devices()
        } catch {
            self.error = Self.message(error)
        }
        phoneKey = try? await model.core.phoneKeyFingerprint()
    }

    func signOut(_ device: DeviceView, _ model: AppModel) async {
        guard !busy else { return }
        busy = true
        error = nil
        message = nil
        defer { busy = false }
        do {
            try await model.core.signOutDevice(deviceId: device.id)
            model.feedback.play(.revoked)
            devices = try await model.core.devices()
            message = "\(untrusted(device.name)) is signed out. It can no longer open your vault or answer requests."
        } catch {
            model.feedback.play(.error)
            self.error = Self.message(error)
        }
    }

    /// Only the approval phone sees and signs out the account's devices.
    static func message(_ error: Error) -> String {
        if case let CoreError.Server(status, _) = error, status == 403 {
            return "Only your approval phone lists and signs out devices. Use this phone for approvals first (Settings, Approval device)."
        }
        return error.userMessage
    }

    static func symbol(_ kind: DeviceKind) -> String {
        switch kind {
        case .phone: "iphone"
        case .computer: "laptopcomputer"
        case .browser: "globe"
        case .other: "app"
        }
    }
}

struct DevicesScreen: View {
    @Environment(AppModel.self) private var model
    @State private var vm = DevicesModel()
    @State private var signingOut: DeviceView?

    var body: some View {
        let me = vm.devices?.first { $0.thisDevice }
        let others = (vm.devices ?? []).filter { !$0.thisDevice }
        let computers = model.connections.filter { $0.keyFingerprint != nil }
        List {
            if vm.message != nil || vm.error != nil {
                Section {
                    if let message = vm.message { FormBanner(text: message, kind: .info).accessibilityIdentifier("devicesMessage") }
                    if let error = vm.error { FormBanner(text: error).accessibilityIdentifier("devicesError") }
                }
                .listRowBackground(Color.clear)
            }
            Section {
                let about = [
                    me?.approval == true ? "Your approval device: requests come here" : nil,
                    vm.phoneKey.map { "Key for reins vault add: \($0)" },
                ].compactMap { $0 }.joined(separator: "\n")
                InfoRow(
                    title: me.map { untrusted($0.name) } ?? "This phone",
                    subtitle: about.isEmpty ? nil : about,
                    symbol: "iphone",
                    tint: Palette.accent
                ) { EmptyView() }
                .accessibilityIdentifier("thisDevice")
                .cardRow()
            } header: {
                GroupHeader("This phone")
            }
            Section {
                if vm.devices == nil && vm.error == nil {
                    InfoRow(title: "Loading…") { EmptyView() }.cardRow()
                } else if others.isEmpty {
                    InfoRow(title: "No other device is signed in to your account.") { EmptyView() }
                        .accessibilityIdentifier("noOtherDevices")
                        .cardRow()
                }
                ForEach(others, id: \.id) { device in
                    HStack(spacing: 10) {
                        InfoRow(
                            title: untrusted(device.name),
                            subtitle: "\(device.platform) · last seen \(TimeText.relative(device.lastSeenAt))" + (device.approval ? "\nApproval device" : ""),
                            symbol: DevicesModel.symbol(device.kind)
                        ) { EmptyView() }
                        Button("Sign out", role: .destructive) { signingOut = device }
                            .buttonStyle(.borderless)
                            .font(RFont.sans(15, .semibold))
                            .disabled(vm.busy)
                            .accessibilityIdentifier("signOutDevice:\(device.id)")
                    }
                    .accessibilityIdentifier("device:\(device.id)")
                    .cardRow()
                }
            } header: {
                GroupHeader("Other phones and apps")
            } footer: {
                GroupFooter("Lost a phone? Sign it out here. It can no longer open your vault or answer requests; what it kept stays encrypted behind its screen lock.")
            }
            Section {
                if computers.isEmpty {
                    InfoRow(title: "No computer is connected.") { EmptyView() }.accessibilityIdentifier("noComputers").cardRow()
                }
                ForEach(computers, id: \.id) { connection in
                    SettingsLinkRow(
                        title: untrusted(connection.label),
                        subtitle: [
                            connection.keyFingerprint.map { "Key \($0)" },
                            connection.lastUsedAt.map { "used \(TimeText.relative($0))" } ?? "never used",
                        ].compactMap { $0 }.joined(separator: " · "),
                        symbol: "laptopcomputer",
                        tint: Palette.secondary,
                        id: "deviceComputer:\(connection.id)"
                    ) { model.push(.connection(connection.id)) }
                }
            } header: {
                GroupHeader("Computers")
            } footer: {
                GroupFooter("A computer's page removes it; on the computer, reins logout does the same.")
            }
        }
        .reinsGrouped()
        .navigationTitle("Devices")
        .animation(.smooth(duration: 0.25), value: vm.devices)
        .task { await vm.load(model) }
        .refreshable { await vm.load(model) }
        .confirmationDialog(
            "Sign out \(untrusted(signingOut?.name ?? ""))?",
            isPresented: Binding(get: { signingOut != nil }, set: { if !$0 { signingOut = nil } }),
            titleVisibility: .visible,
            presenting: signingOut
        ) { device in
            Button("Sign out", role: .destructive) {
                signingOut = nil
                Task { await vm.signOut(device, model) }
            }
        } message: { _ in
            Text("It can no longer open your vault, sync or answer requests. If you find it, sign in on it again.")
        }
        .presentationFeedback(signingOut != nil)
    }
}
