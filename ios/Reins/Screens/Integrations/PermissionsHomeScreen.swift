import SwiftUI

/// Permissions (the Permissions board, 01; quiet): every account, each opening its rules, then the temporary
/// permissions, each opening its page (05) with Pause or Resume and Revoke.
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
    /// The temporary permission whose page is open.
    @State private var openRecord: String?
    private struct PendingChange: Identifiable {
        var record: DelegationView
        var change: DelegationChange
        var id: String { record.id }
    }

    var body: some View {
        List {
            PermTitle("Permissions").permRow(sides: 0)
            PermHeader(
                title: "Accounts", help: "These settings also appear in Connections. Automatic permissions apply only to their selected agents and account. New agents need explicit account access.",
                count: "\(accounts.count)"
            )
            .permRow(sides: 0)
            FlatGroup(inset: 44) {
                ForEach(accounts, id: \.permissionIdentity) { account in
                    Button { app.show(.accountPermissions(service: account.service, account: account.account, temporary: false)) } label: {
                        PermRow(title: untrusted(account.account), mono: true, chevron: true) { ServiceAvatar(service: account.service, size: 32) }
                    }
                    .buttonStyle(PressDimsStyle())
                    .accessibilityLabel("\(untrusted(account.account)), \(serviceName(account.service)) permissions")
                    .accessibilityIdentifier("managePermissions:\(account.service):\(account.account)")
                }
            }
            .permRow(sides: 0)
            PermHeader(
                title: "Temporary", glyph: .timer,
                help: "Open an account with supported automatic permissions above to create a temporary permission. Unknown outcomes still use the reserved budget.",
                count: "\(records.count)"
            )
            .permRow(sides: 0)
            FlatGroup(inset: 44) {
                if records.isEmpty {
                    Text("No temporary permissions").textRole(.body).foregroundStyle(Palette.inkMeta).frame(minHeight: 52)
                        .frame(maxWidth: .infinity, alignment: .leading)
                }
                ForEach(records, id: \.id) { record in
                    Button { openRecord = record.id } label: {
                        PermRow(title: untrusted(record.account), mono: true, value: DelegationText.uses(record), chevron: true) {
                            ServiceAvatar(service: record.service, size: 32)
                        }
                    }
                    .buttonStyle(PressDimsStyle())
                    .accessibilityLabel("\(untrusted(record.title)), \(DelegationText.state(record.state)), \(DelegationText.uses(record))")
                    .accessibilityIdentifier("delegation:\(record.id)")
                }
            }
            .permRow(sides: 0)
            if let error { Banner(untrusted(error), kind: .error).accessibilityIdentifier("delegationError").permRow(top: 14, sides: 16) }
        }
        .integrationList()
        .navigationTitle("")
        .navigationBarTitleDisplayMode(.inline)
        .task {
            activation &+= 1; active = true; busy = false; confirm = nil; epoch = app.accountEpoch; await reload()
            #if DEBUG
            // `-openRecord` (screenshots): the first temporary permission's page.
            if ProcessInfo.processInfo.arguments.contains("-openRecord") { openRecord = records.first?.id }
            #endif
        }
        .onDisappear { if openRecord == nil { active = false; activation &+= 1 } }
        .refreshable { await reload() }
        .navigationDestination(item: $openRecord) { id in
            DelegationPage(
                record: records.first { $0.id == id }, busy: busy, error: error,
                onReload: { Task { await reload() } },
                onChange: { record, change in confirm = PendingChange(record: record, change: change) }
            )
            .confirmationDialog(confirm.map { "\(DelegationText.change($0.change)) temporary permission?" } ?? "", isPresented: Binding(get: { confirm != nil }, set: { if !$0 { confirm = nil } }), titleVisibility: .visible, presenting: confirm) { item in
                Button(DelegationText.change(item.change), role: item.change == .revoke ? .destructive : nil) { confirm = nil; Task { await change(item) } }
                Button("Cancel", role: .cancel) {}
            } message: { item in Text("\(untrusted(item.record.scopeSummary))\n\(untrusted(item.record.account))") }
        }
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
}

/// A temporary permission's words.
enum DelegationText {
    static func state(_ value: DelegationState) -> String {
        switch value {
        case .active: "Active"
        case .paused: "Paused"
        case .expired: "Expired"
        case .revoked: "Revoked"
        }
    }

    static func change(_ value: DelegationChange) -> String {
        switch value {
        case .resume: "Resume"
        case .pause: "Pause"
        case .revoke: "Revoke"
        }
    }

    /// "0/20 uses", or that usage cannot be read.
    static func uses(_ r: DelegationView) -> String { r.usageVerified ? "\(r.used)/\(r.maxUses) uses" : "Usage unavailable" }
}

/// One temporary permission (the Permissions board, 05): the account with its state, what it allows and for whom, its
/// uses, unknown outcomes and expiry; Pause (or Resume) and Revoke, each confirmed and checked by the owner.
private struct DelegationPage: View {
    var record: DelegationView?
    var busy: Bool
    var error: String?
    var onReload: () -> Void
    var onChange: (DelegationView, DelegationChange) -> Void

    var body: some View {
        Group {
            if let record {
                content(record)
            } else {
                EmptyState(symbol: "clock", title: "This permission is gone").frame(maxWidth: .infinity, maxHeight: .infinity)
            }
        }
        .background(Palette.paper.ignoresSafeArea())
        .navigationTitle("")
        .navigationBarTitleDisplayMode(.inline)
        .toolbar {
            ToolbarItem(placement: .topBarTrailing) {
                Button(action: onReload) { GlyphIcon(.refresh, size: 18).foregroundStyle(Palette.ink) }
                    .accessibilityLabel("Reload")
                    .accessibilityIdentifier("delegationReload")
            }
        }
    }

    private func content(_ record: DelegationView) -> some View {
        let tone: Pill.Tone = switch record.state {
        case .active: .success
        case .paused: .neutral
        case .expired: .warning
        case .revoked: .danger
        }
        return List {
            PermTitle(title: untrusted(record.account)) {
                ServiceAvatar(service: record.service, size: 32)
            } trail: {
                Pill(DelegationText.state(record.state), tone: tone)
            }
            .permRow(sides: 0)
            PermHeader(title: "Temporary", glyph: .timer).permRow(sides: 0)
            FlatGroup(inset: 44) {
                PermRow(title: untrusted(record.title)) { GlyphTile(.person) }
                PermRow(title: "Uses", value: record.usageVerified ? "\(record.used) / \(record.maxUses)" : "Unavailable") { GlyphTile(.card) }
                PermRow(title: "Unknown outcomes", value: "\(record.unresolved)") { GlyphTile(.help) }
                PermRow(title: "Expires", value: Date(timeIntervalSince1970: TimeInterval(record.expiresAt)).formatted(.dateTime.month(.abbreviated).day().hour().minute())) { GlyphTile(.timer) }
            }
            .permRow(sides: 0)
            VStack(alignment: .leading, spacing: 8) {
                PermLine(untrusted(record.scopeSummary))
                PermLine(untrusted(record.effectiveReason))
                ForEach(Array(record.restrictions.enumerated()), id: \.offset) { _, r in
                    PermWarning("\(PermissionCopy.controller(r.controller)): \(untrusted(r.reason))")
                }
                if !record.usageVerified { PermWarning("Usage and quota unavailable.") }
                if let error { Banner(untrusted(error), kind: .error).accessibilityIdentifier("delegationError") }
            }
            .permRow(top: 14)
        }
        .integrationList()
        .accessibilityIdentifier("delegationPage:\(record.id)")
        .safeAreaInset(edge: .bottom, spacing: 0) {
            if record.state != .revoked, record.state != .expired {
                PermFoot {
                    if record.state == .active {
                        changeButton(record, .pause, kind: .soft, glyph: .pause)
                    } else if record.state == .paused {
                        changeButton(record, .resume, kind: .soft, glyph: .play)
                            .disabled(!record.usageVerified || record.restrictions.contains(where: { $0.controller == .conflict || $0.controller == .unsupportedAdapter }))
                    }
                    changeButton(record, .revoke, kind: .danger, glyph: nil)
                }
            }
        }
    }

    private func changeButton(_ record: DelegationView, _ change: DelegationChange, kind: SignalButtonKind, glyph: Glyph?) -> some View {
        Button { onChange(record, change) } label: { PermGlyphLabel(DelegationText.change(change), glyph: glyph) }
            .buttonStyle(SignalButtonLook(kind: kind))
            .frame(maxWidth: .infinity)
            .disabled(busy)
            .accessibilityIdentifier("\(DelegationText.change(change).lowercased())Delegation:\(record.id)")
    }
}

private extension AccountView {
    var permissionIdentity: String { "\(service)/\(account)" }
}
