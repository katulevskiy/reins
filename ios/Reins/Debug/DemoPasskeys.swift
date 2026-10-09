import Foundation

/// The demo's passkey sheet: it answers after a moment, as a password manager with PRF would. Launch arguments:
/// `-demoPasskeyUnsupported` (a password manager without PRF), `-demoPasskeyLatePrf` (it gives the PRF output only as
/// the new passkey is used), `-demoPasskeyCancel` (the sheet is closed), `-demoPasskeyNoneHere` (none of the account's
/// passkeys is on this phone).
@MainActor
final class DemoPasskeys: PasskeyPrompting {
    private let arguments: [String]
    private var made = 1

    init(arguments: [String] = ProcessInfo.processInfo.arguments) {
        self.arguments = arguments
    }

    func create(_ options: VaultPasskeyOptions) async -> PasskeyResult {
        try? await Task.sleep(for: .milliseconds(300))
        if arguments.contains("-demoPasskeyCancel") { return .cancelled }
        if arguments.contains("-demoPasskeyUnsupported") { return .unsupported }
        made += 1
        let id = Data("demo-passkey-\(made)".utf8)
        return .passkey(credentialId: id, prf: arguments.contains("-demoPasskeyLatePrf") ? nil : DemoReinsCore.demoPrfOutput)
    }

    func get(_ options: VaultPasskeyOptions, allowed: [Data]) async -> PasskeyResult {
        try? await Task.sleep(for: .milliseconds(300))
        if arguments.contains("-demoPasskeyCancel") { return .cancelled }
        if arguments.contains("-demoPasskeyNoneHere") { return .unsupported }
        guard let id = allowed.first else { return .unsupported }
        return .passkey(credentialId: id, prf: DemoReinsCore.demoPrfOutput)
    }
}
