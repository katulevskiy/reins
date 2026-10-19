import SwiftUI

struct PermissionsHomeScreen: View {
    @Environment(AppModel.self) private var app
    @State private var accounts: [AccountView] = []
    @State private var records: [DelegationView] = []
    @State private var error: String?
    @State private var busy = false
    @State private var confirm: PendingChange?
    @State private var epoch: UInt64?
    @State private var active = false
    @State private var activation: UInt64 = 0
    private struct PendingChange: Identifiable {
        var record: DelegationView
        var change: DelegationChange
        var id: String { record.id }
    }
    var body: some View {
        List {
            Section {
                ForEach(accounts, id: \.permissionIdentity) { account in
                    Button { app.show(.accountPermissions(service: account.service, account: account.account, temporary: false)) } label: {
                        VStack(alignment: .leading, spacing: 6) {
                            Text(untrusted(account.account)).foregroundStyle(Palette.ink)
                            Text("View \(serviceName(account.service)) permissions").foregroundStyle(Palette.ink2)
                        }
                    }.accessibilityIdentifier("managePermissions:\(account.service):\(account.account)")
                }
            } header: { Text("Accounts") } footer: { Text("These settings also appear in Connections. Each permission applies only to its selected agents and account; future agents do not inherit access.") }
            Section {
                if records.isEmpty { Text("No temporary permissions") }
                ForEach(records, id: \.id) { record in
                    VStack(alignment: .leading, spacing: 12) {
                        Text(untrusted(record.title)).textRole(.title)
                        Text("\(stateTitle(record.state)) · \(untrusted(record.account))")
                        Text(untrusted(record.scopeSummary))
                        Text((record.usageVerified ? "\(record.used) of \(record.maxUses) uses · \(record.unresolved) unresolved outcomes" : "Usage and quota unavailable") + "\nExpires \(PermissionCopy.expiry(record.expiresAt))").font(.footnote).foregroundStyle(Palette.ink2)
                        Text(untrusted(record.effectiveReason)).foregroundStyle(Palette.ink2)
                        PermissionRestrictions(restrictions: record.restrictions)
                        if record.state == .active { changeButton(record, .pause) }
                        if record.state == .paused { changeButton(record, .resume) }
                        if record.state != .revoked, record.state != .expired { changeButton(record, .revoke) }
                    }.padding(.vertical, 8).accessibilityIdentifier("delegation:\(record.id)")
                }
            } header: { Text("Temporary permissions") } footer: { Text("Open an account with supported automatic permissions above to create a temporary permission. Unknown outcomes still use the reserved budget.") }
            if let error { Banner(untrusted(error), kind: .error).accessibilityIdentifier("delegationError") }
            ActionButton(title: "Reload", kind: .secondary, busy: busy, lineLimit: nil) { Task { await reload() } }
        }
        .integrationList().navigationTitle("Permissions")
        .task { activation &+= 1; active = true; busy = false; confirm = nil; epoch = app.accountEpoch; await reload() }
        .onDisappear { active = false; activation &+= 1 }
        .refreshable { await reload() }
        .confirmationDialog(confirm.map { "\(changeTitle($0.change)) temporary permission?" } ?? "", isPresented: Binding(get: { confirm != nil }, set: { if !$0 { confirm = nil } }), titleVisibility: .visible, presenting: confirm) { item in
            Button(changeTitle(item.change)) { confirm = nil; Task { await change(item) } }
            Button("Cancel", role: .cancel) {}
        } message: { item in Text("\(untrusted(item.record.scopeSummary))\n\(untrusted(item.record.account))") }
    }
    private func changeButton(_ record: DelegationView, _ change: DelegationChange) -> some View {
        ActionButton(title: changeTitle(change), kind: .secondary, lineLimit: nil) { confirm = PendingChange(record: record, change: change) }
             .disabled(busy || (change == .resume && (!record.usageVerified || record.restrictions.contains(where: { $0.controller == .conflict || $0.controller == .unsupportedAdapter })))).accessibilityIdentifier("\(changeTitle(change).lowercased())Delegation:\(record.id)")
    }
    private func reload() async {
        guard active, let epoch, app.accountEpoch == epoch else { return }
        let token = activation
        do {
            let services = try await app.core.services()
            let gmail = try await app.core.accounts()
            let all = services.flatMap(\.accounts) + gmail
            let rows = try await app.core.delegations()
            guard active, activation == token, app.accountEpoch == epoch else { return }
            var keys = Set<String>()
            accounts = all.filter { keys.insert("\($0.service)/\($0.account)").inserted }
            records = rows
            error = nil
        } catch { if active, activation == token, app.accountEpoch == epoch { self.error = error.userMessage } }
    }
    private func change(_ item: PendingChange) async {
        guard active, let epoch, app.accountEpoch == epoch, !busy else { return }
        busy = true
        let token = activation
        let owner = PermissionsOwnerCheck(app, stillActive: { active && activation == token })
        defer { if active, activation == token, app.accountEpoch == epoch { busy = false } }
        do {
            _ = try await app.core.changeDelegation(id: item.record.id, expectedRevision: item.record.revision, change: item.change, owner: owner)
            if active, activation == token, app.accountEpoch == epoch { await reload() }
        } catch {
            guard active, activation == token, app.accountEpoch == epoch else { return }
            if owner.last != .cancelled { self.error = error.userMessage + " Reload before trying again." }
        }
    }
    private func stateTitle(_ value: DelegationState) -> String { switch value { case .active: "Active"; case .paused: "Paused"; case .expired: "Expired"; case .revoked: "Revoked" } }
    private func changeTitle(_ value: DelegationChange) -> String { switch value { case .resume: "Resume"; case .pause: "Pause"; case .revoke: "Revoke" } }
}

private extension AccountView {
    var permissionIdentity: String { "\(service)/\(account)" }
}
