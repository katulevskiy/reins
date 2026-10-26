import SwiftUI
import UIKit

/// Settings > Devices (the Android app's `DevicesViewModel`): the account's devices, and signing one out (a lost
/// phone).
@Observable
@MainActor
final class DevicesModel {
    /// Nil until first read.
    private(set) var devices: [DeviceView]?
    /// The digits of this phone's key, which `reins vault add` asks for once.
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
            message = "\(untrustedName(device.name)) is signed out. It can no longer open your vault or answer requests, and cannot sign in again as it is."
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

    static func glyph(_ kind: DeviceKind) -> Glyph {
        switch kind {
        case .phone: .phone
        case .computer: .laptop
        case .browser: .globe
        case .other: .devices
        }
    }
}

/// The page (the Settings board, 02), quiet: this phone and its key, the other phones (Sign out asks for the recovery
/// code), the computers.
struct DevicesScreen: View {
    @Environment(AppModel.self) private var model
    @Environment(\.feedback) private var feedback
    @State private var vm = DevicesModel()
    @State private var signingOut: DeviceView?
    @State private var proof = ""

    var body: some View {
        let me = vm.devices?.first { $0.thisDevice }
        let others = (vm.devices ?? []).filter { !$0.thisDevice }
        let computers = model.connections.filter { $0.keyFingerprint != nil }
        QuietPage("Devices") {
            if let message = vm.message { QuietBanner(text: message, kind: .info).accessibilityIdentifier("devicesMessage") }
            if let error = vm.error { QuietBanner(text: error).accessibilityIdentifier("devicesError") }

            SectionHeader("This phone", quiet: true)
            FlatGroup(inset: 0) {
                TileRow(me.map { untrustedName($0.name) } ?? "This phone", glyph: .phone, chevron: false) {
                    if me?.approval == true { ApprovesTag() }
                }
                .flatRow()
                .accessibilityElement(children: .combine)
                .accessibilityIdentifier("thisDevice")
                if let key = vm.phoneKey {
                    TileRow("Key", chevron: false) {
                        GlyphTile(.terminal)
                    } trailing: {
                        Text(key)
                            .font(RFont.mono(13))
                            .foregroundStyle(Palette.ink)
                            .lineLimit(1)
                            .environment(\.layoutDirection, .leftToRight)
                            .accessibilityLabel("Key \(key), for reins vault add")
                        RowIconButton(glyph: .copy, label: "Copy key") {
                            UIPasteboard.general.string = key
                            feedback.play(.copied)
                            model.notice = "Key copied."
                        }
                        .padding(.trailing, -10)
                    }
                    .flatRow()
                    .accessibilityIdentifier("phoneKey")
                }
            }

            QuietHelpHeader(title: "Other phones", help: "Lost a phone? Sign it out here: the app on it can no longer open your vault or answer requests, nor sign in again as it is. Whoever has it and its passcode could still read your recovery code on it and sign in with a new install; reset the vault if that may happen.")
            FlatGroup {
                if vm.devices == nil && vm.error == nil {
                    TileRow("Loading…", glyph: .phone, chevron: false).flatRow()
                } else if others.isEmpty {
                    TileRow("None", glyph: .phone, chevron: false)
                        .flatRow()
                        .accessibilityLabel("No other device is signed in to your account.")
                        .accessibilityIdentifier("noOtherDevices")
                }
                ForEach(others, id: \.id) { device in
                    TileRow(untrustedName(device.name, limit: 40), value: TimeText.day(device.lastSeenAt), chevron: false) {
                        GlyphTile(DevicesModel.glyph(device.kind))
                    } trailing: {
                        if device.approval { ApprovesTag() }
                        Button("Sign out", role: .destructive) { signingOut = device }
                            .buttonStyle(.borderless)
                            .font(RFont.sans(15, .semibold))
                            .foregroundStyle(Palette.danger)
                            .frame(minHeight: 44)
                            .disabled(vm.busy)
                            .accessibilityLabel("Sign out \(untrustedName(device.name, limit: 40))")
                            .accessibilityIdentifier("signOutDevice:\(device.id)")
                    }
                    .flatRow()
                    .accessibilityElement(children: .contain)
                    .accessibilityIdentifier("device:\(device.id)")
                }
            }

            QuietHelpHeader(title: "Computers", help: "A computer's page removes it; on the computer, reins logout does the same.", count: computers.isEmpty ? nil : "\(computers.count)")
            FlatGroup {
                if computers.isEmpty {
                    TileRow("None", glyph: .laptop, chevron: false)
                        .flatRow()
                        .accessibilityLabel("No computer is connected.")
                        .accessibilityIdentifier("noComputers")
                }
                ForEach(computers, id: \.id) { connection in
                    Button {
                        model.push(.connection(connection.id))
                        feedback.defaultTap()
                    } label: {
                        TileRow(untrustedName(connection.label, limit: 40), value: connection.lastUsedAt.map { SettingsText.compactAgo($0) } ?? "Never") {
                            ConnectionAvatar(connection: connection, size: 32)
                        } trailing: { EmptyView() }
                        .flatRow()
                    }
                    .buttonStyle(.plain)
                    .accessibilityLabel(untrustedName(connection.label, limit: 40))
                    .accessibilityValue(connection.lastUsedAt.map { "used \(TimeText.relative($0))" } ?? "never used")
                    .accessibilityIdentifier("deviceComputer:\(connection.id)")
                }
            }
        }
        .privacySensitive()
        .animation(.smooth(duration: 0.25), value: vm.devices)
        .task { await vm.load(model) }
        .refreshable { await vm.load(model) }
        .alert(
            "Sign out \(untrustedName(signingOut?.name ?? "", limit: 40))?",
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
            Text("\(device.platform) · signed in \(TimeText.relative(device.createdAt)) · last seen \(TimeText.relative(device.lastSeenAt)). The app on it can no longer open your vault, sync or answer requests, and cannot sign in again as it is. Someone with its passcode could still read your recovery code on it (see I lost my phone in the help). Type your recovery code (or master password) to confirm.")
        }
        .presentationFeedback(signingOut != nil)
    }
}

/// The phone the requests go to: the only green on the page.
private struct ApprovesTag: View {
    var body: some View {
        StatusTag(text: "Approves", glyph: .shieldCheck).accessibilityLabel("Approval phone")
    }
}
