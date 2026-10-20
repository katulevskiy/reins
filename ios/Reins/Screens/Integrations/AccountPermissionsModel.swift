import Foundation
import Observation

/// The typed account editor; preset rules, admission and effective decisions belong to the core.
@Observable
@MainActor
final class AccountPermissionsModel {
    let service: String
    let account: String
    let temporary: Bool
    private(set) var view: AccountPermissionsView?
    private(set) var draft: AccountPermissionsDraft?
    private(set) var preview: PermissionPreview?
    private(set) var options: PermissionScopeOptions?
    private(set) var optionsLoading = false
    private(set) var loading = true
    private(set) var busy = false
    private(set) var changedElsewhere = false
    private(set) var saved = false
    private(set) var issued: DelegationView?
    private(set) var hasChosenPreset = false
    private(set) var error: String?
    var canEdit: Bool { view?.availablePresets.isEmpty == false }

    private let app: AppModel
    private let epoch: UInt64
    private var optionsRevision: UInt64 = 0
    private var active = true
    private var current: Bool { active && app.accountEpoch == epoch }
    var currentForSetup: Bool { current }
    func deactivate() { active = false; optionsRevision &+= 1 }

    init(app: AppModel, service: String, account: String, temporary: Bool = false) {
        self.app = app
        self.service = service
        self.account = account
        self.temporary = temporary
        epoch = app.accountEpoch
    }

    /// Reload is explicit after a revision conflict; the draft is never silently rebased.
    func reload() async {
        await operation {
            let view = try await app.core.accountPermissions(service: service, account: account)
            guard !view.availablePresets.isEmpty else {
                guard current else { return }
                self.view = view
                self.draft = nil
                preview = nil
                saved = false
                hasChosenPreset = false
                changedElsewhere = false
                return
            }
            let draft: AccountPermissionsDraft
            if temporary { draft = try await app.core.delegationDraft(service: service, account: account).asAccountDraft() }
            else { draft = try await app.core.accountPermissionDraft(service: service, account: account, preset: .custom) }
            guard current else { return }
            self.view = view
            preview = nil
            saved = false
            hasChosenPreset = false
            changedElsewhere = view.revision != draft.expectedRevision
            self.draft = changedElsewhere ? nil : draft
            if changedElsewhere { error = "Permissions changed while loading. Reload to see the current settings." }
        }
    }

    func preset(_ preset: AccountPermissionPreset) async {
        guard canEdit else { return }
        // Customize keeps the core-produced unsaved preset and the owner's explicit selected agents.
        if preset == .custom, hasChosenPreset { edit(customize: true) { _ in }; return }
        await operation {
            let next = try await app.core.accountPermissionDraft(service: service, account: account, preset: preset)
            guard current else { return }
            preview = nil
            guard next.expectedRevision == draft?.expectedRevision else {
                changedElsewhere = true
                return
            }
            draft = next
            saved = false
            hasChosenPreset = true
        }
    }

    private func edit(customize: Bool = false, _ change: (inout AccountPermissionsDraft) -> Void) {
        guard current, canEdit, !busy, !changedElsewhere, var draft else { return }
        change(&draft)
        if customize { draft.preset = .custom }
        self.draft = draft
        hasChosenPreset = true
        preview = nil
        saved = false
        error = nil
    }

    func include(_ capability: PermissionCapability, selected: Bool) {
        guard let capabilityView = view?.capabilities.first(where: { $0.capability == capability }) else { return }
        edit { draft in
            draft.rules.removeAll { $0.capability == capability }
            if selected { draft.rules.append(CapabilityPermissionInput(capability: capability, mode: .auto, scope: capabilityView.scope)) }
        }
    }

    func mode(_ capability: PermissionCapability, _ mode: PermissionMode) {
        edit(customize: true) { draft in
            guard let i = draft.rules.firstIndex(where: { $0.capability == capability }) else { return }
            draft.rules[i].mode = mode
        }
    }

    func scope(_ capability: PermissionCapability, _ scope: PermissionScopeSelection) {
        edit(customize: true) { draft in
            guard let i = draft.rules.firstIndex(where: { $0.capability == capability }) else { return }
            draft.rules[i].scope = scope
        }
    }

    func agent(_ id: String, selected: Bool) {
        edit { draft in
            draft.agentIds.removeAll { $0 == id }
            if selected { draft.agentIds.append(id) }
            let existing = Set(view?.agents.filter(\.hasAccountAccess).map(\.connectionId) ?? [])
            draft.grantAccountAccess = draft.agentIds.contains { !existing.contains($0) }
        }
    }

    func expiresAt(_ value: Int64?) { edit { $0.expiresAt = value } }
    func remainingUses(_ value: UInt64?) { edit { $0.maxUses = value } }
    func discardPreview() { preview = nil }
    func closeOptions() { optionsRevision &+= 1; options = nil; optionsLoading = false }

    /// The core normalizes typed addresses and returns account-bound canonical picker IDs.
    func scopeOptions(_ capability: PermissionCapability, query: String) async {
        guard current, canEdit else { return }
        optionsRevision &+= 1
        let revision = optionsRevision
        optionsLoading = true
        do {
            try await Task.sleep(for: .milliseconds(250))
            let options = try await app.core.permissionScopeOptions(service: service, account: account, capability: capability, query: query, limit: 30)
            guard current, revision == optionsRevision else { return }
            self.options = options
            optionsLoading = false
            error = nil
        } catch is CancellationError {
        } catch {
            if current, revision == optionsRevision { optionsLoading = false; self.error = error.userMessage }
        }
    }

    func reviewChanges() async {
        await operation {
            guard canEdit, let draft else { return }
            let result = temporary ? try await app.core.previewDelegation(draft: draft.asDelegationDraft()) : try await app.core.previewAccountPermissions(draft: draft)
            if current { preview = result }
        }
    }

    /// Saves the exact previewed draft. A failed save leaves the connected account untouched.
    func save() async {
        guard canEdit, let draft, preview?.revision == draft.expectedRevision, !changedElsewhere else { return }
        let owner = PermissionsOwnerCheck(app, stillActive: { [weak self] in self?.current == true })
        await operation(owner: owner) {
            if temporary {
                let record = try await app.core.issueDelegation(draft: draft.asDelegationDraft(), owner: owner)
                if current { issued = record }
            }
            else {
                let result = try await app.core.saveAccountPermissions(draft: draft, owner: owner)
                if current { view = result }
            }
            guard current else { return }
            preview = nil
            saved = true
        }
    }

    private func operation(owner: PermissionsOwnerCheck? = nil, _ body: () async throws -> Void) async {
        guard current, !busy else { return }
        busy = true
        error = nil
        defer { if current { busy = false; loading = false } }
        do {
            try await body()
        } catch is CancellationError {
        } catch {
            guard current else { return }
            let latest = try? await app.core.accountPermissions(service: service, account: account)
            guard current, !Task.isCancelled else { return }
            switch owner?.last {
            case .cancelled: self.error = nil
            case .unavailable: self.error = "Set a passcode on this iPhone to change permissions."
            default: self.error = error.userMessage
            }
            preview = nil
            if let latest, let draft, latest.revision != draft.expectedRevision { changedElsewhere = true }
        }
    }
}

/// Records a cancelled owner check without treating cancellation as a policy failure.
@MainActor
final class PermissionsOwnerCheck: OwnerCheck {
    private let app: AppModel
    private let epoch: UInt64
    private let stillActive: @MainActor () -> Bool
    private(set) var last: OwnerPrompt.Result?

    init(_ app: AppModel, stillActive: @escaping @MainActor () -> Bool = { true }) { self.app = app; epoch = app.accountEpoch; self.stillActive = stillActive }

    func confirm(reason: String) async throws -> Bool {
        guard app.accountEpoch == epoch, stillActive() else { return false }
        last = await OwnerPrompt.confirm(app, reason: reason)
        return app.accountEpoch == epoch && stillActive() && last == .confirmed
    }
}

private extension DelegationDraft {
    func asAccountDraft() -> AccountPermissionsDraft {
        AccountPermissionsDraft(service: service, account: account, expectedRevision: expectedRevision, preset: .custom, agentIds: agentIds, grantAccountAccess: grantAccountAccess, expiresAt: expiresAt, maxUses: maxUses, rules: rules)
    }
}
private extension AccountPermissionsDraft {
    func asDelegationDraft() throws -> DelegationDraft {
        guard let expiresAt, let maxUses else { throw CoreError.Invalid(reason: "Set an expiry and use limit for this temporary permission.") }
        return DelegationDraft(service: service, account: account, expectedRevision: expectedRevision, agentIds: agentIds, grantAccountAccess: grantAccountAccess, rules: rules, expiresAt: expiresAt, maxUses: maxUses)
    }
}
