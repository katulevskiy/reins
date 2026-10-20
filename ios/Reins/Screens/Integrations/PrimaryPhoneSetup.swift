import Observation
import SwiftUI
import UIKit

/// Explicit account-wide role creation. Reading status never creates, recovers, or transfers authority.
@Observable
@MainActor
final class PrimaryPhoneSetupModel {
    private(set) var checking = true
    private(set) var busy = false
    private(set) var role: PhoneRole?
    private(set) var hasRoster = false
    private(set) var primaryName: String?
    private(set) var error: String?
    private(set) var complete = false
    private let app: AppModel
    private let epoch: UInt64
    private let parentCurrent: @MainActor () -> Bool
    private var active = true
    private var revision: UInt64 = 0
    var canCreate: Bool { !checking && !busy && role == .legacy && !hasRoster }

    init(app: AppModel, parentCurrent: @escaping @MainActor () -> Bool) {
        self.app = app; epoch = app.accountEpoch; self.parentCurrent = parentCurrent
    }
    private func current(_ token: UInt64) -> Bool { active && token == revision && parentCurrent() && app.accountEpoch == epoch }
    func deactivate() { active = false; revision &+= 1 }
    func cancelAttempt() { revision &+= 1; busy = false }

    func check() async {
        let token = revision
        guard current(token), !busy else { return }
        checking = true; error = nil
        do {
            let roster = try await app.core.rosterSummary(refresh: true)
            let role = try await app.core.phoneRole()
            guard current(token) else { return }
            checked(roster, role)
        } catch {
            if current(token) { checking = false; self.error = error.userMessage }
        }
    }

    func enable(backupConfirmed: Bool, proof: String?, onPrimary: () async -> Void) async {
        let token = revision
        guard current(token), backupConfirmed, canCreate else { return }
        busy = true; error = nil
        defer { if current(token) { busy = false } }
        let owner = PermissionsOwnerCheck(app, stillActive: { [weak self] in self?.current(token) == true })
        do {
            // Preflight never creates authority. The bound core callback rechecks this exact attempt after prepare.
            let roster = try await app.core.rosterSummary(refresh: true)
            let role = try await app.core.phoneRole()
            guard current(token) else { return }
            guard roster == nil, role == .legacy else {
                checked(roster, role)
                if role == .primary, roster?.members.contains(where: { $0.primary && $0.thisPhone }) == true { complete = true; await onPrimary() }
                return
            }
            // Core checks the fresh server/local roster after owner confirmation, before creating authority.
            let result = try await app.core.rosterCreateAuthorized(deviceName: UIDevice.current.name, codeOrPassword: proof?.isEmpty == false ? proof : nil, owner: owner)
            guard current(token) else { return }
            checked(result, result.role)
            guard result.role == .primary, result.members.contains(where: { $0.primary && $0.thisPhone }) else {
                error = "This phone was not confirmed as the primary. Reload its status before trying again."
                return
            }
            complete = true
            await onPrimary()
        } catch {
            if current(token) {
                switch owner.last {
                case .cancelled: self.error = nil
                case .unavailable: self.error = "Set a passcode on this iPhone first."
                default: self.error = error.userMessage
                }
            }
        }
    }

    private func checked(_ roster: RosterSummary?, _ role: PhoneRole) {
        checking = false; self.role = role; hasRoster = roster != nil
        primaryName = roster?.members.first(where: { $0.primary })?.name
    }
}

struct PrimaryPhoneSetupSection: View {
    var editor: AccountPermissionsModel
    @Environment(AppModel.self) private var app
    @State private var setup: PrimaryPhoneSetupModel?
    @State private var showing = false
    @State private var sheetSetup: PrimaryPhoneSetupModel?
    @State private var backupConfirmed = false
    @State private var enterProof = false
    @State private var proof = ""

    var body: some View {
        Section("Automatic permissions on this phone") {
            if let setup {
                if setup.checking { ProgressView("Checking this phone…") }
                else if setup.canCreate {
                    Text("A verified primary phone is required before automatic permissions can be enabled.").font(.footnote).foregroundStyle(Palette.ink2)
                    ActionButton(title: "Enable automatic permissions on this phone", lineLimit: nil) { sheetSetup = setup; showing = true }.accessibilityIdentifier("primarySetupOpen")
                } else if setup.role == .primary {
                    Text("This phone is already the primary. Current policy and history requirements still apply.").font(.footnote).foregroundStyle(Palette.ink2)
                    ActionButton(title: "Reload permissions", kind: .secondary, lineLimit: nil) { Task { await editor.reload() } }.accessibilityIdentifier("primarySetupReload")
                } else if setup.role == .viewer || setup.role == .removed || setup.hasRoster {
                    Text("Use your primary phone\(setup.primaryName.map { " (\(untrusted($0)))" } ?? "") to manage automatic permissions. This setup cannot take over another phone's role.").font(.footnote).accessibilityIdentifier("primarySetupGuidance")
                }
                if !showing, let error = setup.error {
                    Banner(untrusted(error), kind: .error).accessibilityIdentifier("primarySetupError")
                    ActionButton(title: "Check again", kind: .secondary, lineLimit: nil) { Task { await setup.check() } }
                }
            }
        }
        .task(id: "\(ObjectIdentifier(editor)):\(app.accountEpoch)") {
            close(); setup?.deactivate()
            let model = PrimaryPhoneSetupModel(app: app, parentCurrent: { [weak editor] in editor?.currentForSetup == true })
            setup = model
            await model.check()
        }
        .onDisappear { close(); setup?.deactivate() }
        .sheet(isPresented: $showing) {
            if let setup = sheetSetup {
                NavigationStack {
                    List {
                        Section {
                            Text("This makes this phone the primary approval phone for your whole Reins account, including its other integrations. It creates protected recovery information. It does not allow any agent action by itself.")
                            Toggle("I have saved my recovery code or master password.", isOn: $backupConfirmed).disabled(setup.busy).accessibilityIdentifier("primarySetupBackup")
                            if enterProof { SecureField("Recovery code or master password", text: $proof).textInputAutocapitalization(.never).autocorrectionDisabled().disabled(setup.busy).accessibilityIdentifier("primarySetupProof") }
                            else { ActionButton(title: "Use a recovery code or master password", kind: .secondary, lineLimit: nil) { enterProof = true }.disabled(setup.busy) }
                            if let error = setup.error { Banner(untrusted(error), kind: .error).accessibilityIdentifier("primarySetupError") }
                            if !setup.canCreate, !setup.busy { Text("The phone's role changed. Close this screen and check its current status.").font(.footnote) }
                            ActionButton(title: "Make this my primary phone", busy: setup.busy, lineLimit: nil) {
                                let entered = proof; proof = ""
                                Task { await setup.enable(backupConfirmed: backupConfirmed, proof: entered) { close(setup); await editor.reload() } }
                            }.disabled(!backupConfirmed || !setup.canCreate).accessibilityIdentifier("primarySetupConfirm")
                            ActionButton(title: "Cancel", kind: .secondary, lineLimit: nil) { close(setup) }.accessibilityIdentifier("primarySetupCancel")
                        }
                    }.integrationList().navigationTitle("Enable automatic permissions").navigationBarTitleDisplayMode(.inline)
                }.privacyCover().accessibilityIdentifier("primarySetupDialog").onDisappear { close(setup) }
            }
        }
    }
    private func close() { proof = ""; enterProof = false; backupConfirmed = false; showing = false; sheetSetup?.cancelAttempt(); sheetSetup = nil }
    private func close(_ presented: PrimaryPhoneSetupModel) {
        presented.cancelAttempt()
        if sheetSetup === presented { close() }
    }
}
