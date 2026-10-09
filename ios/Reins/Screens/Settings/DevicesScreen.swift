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

    /// Signs `device` out with the recovery code or master password the user typed now (never one the phone keeps).
    func signOut(_ device: DeviceView, proof: String, _ model: AppModel) async {
        guard !busy else { return }
        busy = true
        error = nil
        message = nil
        defer { busy = false }
        do {
            try await model.core.signOutDevice(deviceId: device.id, codeOrPassword: proof)
            model.feedback.play(.revoked)
            devices = try await model.core.devices()
            message = "\(untrusted(device.name)) is signed out. It can no longer open your vault or answer requests, and cannot sign in again as it is."
        } catch {
            model.feedback.play(.error)
            self.error = Self.message(error)
        }
    }

    /// Only the approval phone sees and signs out the account's devices.
    static func message(_ error: Error) -> String {
        if case let CoreError.Server(status, _) = error, status == 403 {
            return "Devices are listed and signed out from your approval phone, the one that receives the requests."
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
    @State private var proof = ""

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
                GroupFooter("Lost a phone? Sign it out here. It can no longer open your vault or answer requests, nor sign in again as it is. Whoever has it and its passcode could still read what it kept, your recovery code included.")
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
        .privacySensitive()
        .animation(.smooth(duration: 0.25), value: vm.devices)
        .task { await vm.load(model) }
        .refreshable { await vm.load(model) }
        .alert(
            "Sign out \(untrusted(signingOut?.name ?? ""))?",
            isPresented: Binding(get: { signingOut != nil }, set: { if !$0 { signingOut = nil; proof = "" } }),
            presenting: signingOut
        ) { device in
            SecureField("Recovery code or master password", text: $proof)
                .accessibilityIdentifier("signOutProof")
            Button("Sign out", role: .destructive) {
                let typed = proof
                proof = ""
                signingOut = nil
                Task { await vm.signOut(device, proof: typed, model) }
            }
            .disabled(proof.trimmingCharacters(in: .whitespaces).isEmpty)
            Button("Cancel", role: .cancel) {
                proof = ""
                signingOut = nil
            }
        } message: { device in
            // Two phones may share a name: say which one this is.
            Text("\(device.platform) · signed in \(TimeText.relative(device.createdAt)) · last seen \(TimeText.relative(device.lastSeenAt)). It can no longer open your vault, sync or answer requests, and cannot sign in again as it is. Type your recovery code (or master password) to confirm.")
        }
        .presentationFeedback(signingOut != nil)
    }
}
