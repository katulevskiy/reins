import Foundation

/// Synthetic native fixtures only; production policy factories and validation live in reins-core.
enum DemoPermissions {
    static func view(_ service: String, _ account: String, revision: UInt64 = 1) -> AccountPermissionsView {
        let gmail = service == "gmail"
        let modes: [PermissionMode] = gmail ? [.ask, .auto, .block] : []
        let capabilities: [PermissionCapability] = gmail ? [.mailSearch, .mailRead, .mailSend] : [.integrationRead, .integrationWrite]
        return AccountPermissionsView(service: service, account: account, accountId: "\(service)/\(account)", revision: revision, managed: gmail, conflicted: false,
            capabilities: capabilities.map { capability in
                let scope: PermissionScopeKind = capability == .mailSend ? .selectedRecipients : .allInAccount
                let title = capability == .mailSearch ? "Search mail" : capability == .mailRead ? "Read mail" : capability == .mailSend ? "Send mail" : "Integration operations"
                return PermissionCapabilityView(capability: capability, title: title, description: "Only this account and the selected agents are included.", supportedModes: modes, supportedScopes: capability == .mailSend ? [scope] : [scope, .selectedResources], configuredMode: gmail ? .ask : nil, scope: PermissionScopeSelection(kind: scope, ids: []), scopeSummary: gmail ? "Review required for selected agents" : "Existing permissions apply", effectiveReason: gmail ? "New account: review required." : "Automatic permissions are not supported for this integration.", restrictions: gmail ? [] : [PermissionRestriction(controller: .unsupportedAdapter, policyId: nil, reason: "Automatic permissions are not supported for this integration.")])
            }, agents: [PermissionAgentView(connectionId: "c1", label: "Claude Code", hasAccountAccess: true, selected: true), PermissionAgentView(connectionId: "c2", label: "Personal agent", hasAccountAccess: false, selected: false)], availablePresets: gmail ? [.reviewEveryRequest, .readAssist, .custom] : [], expiresAt: nil, maximumDurationSecs: 604800, delegationMaximumDurationSecs: 86400, used: 3, usageVerified: true, remainingUses: nil, scopeSummary: gmail ? "Review required. Existing grants and mandatory restrictions still apply." : "Existing grants and explicit restrictions remain in effect; account defaults are read-only.", restrictions: [])
    }
    static func draft(_ view: AccountPermissionsView, _ preset: AccountPermissionPreset) -> AccountPermissionsDraft {
        AccountPermissionsDraft(service: view.service, account: view.account, expectedRevision: view.revision, preset: preset, agentIds: view.agents.filter(\.selected).map(\.connectionId), grantAccountAccess: false,
            expiresAt: preset == .readAssist ? Int64(Date().timeIntervalSince1970) + 3600 : view.expiresAt, maxUses: preset == .readAssist ? 20 : view.remainingUses,
            rules: view.capabilities.map { CapabilityPermissionInput(capability: $0.capability, mode: preset == .readAssist && $0.capability != .mailSend ? .auto : $0.configuredMode ?? ($0.supportedModes.contains(.ask) ? .ask : .block), scope: $0.scope) })
    }
    static func preview(_ revision: UInt64) -> PermissionPreview {
        PermissionPreview(revision: revision, summary: "Selected agents may search and read this account within the displayed scope and limits. Sending remains subject to review unless explicitly included.", warnings: ["Sensitive-data protections and other review requirements still apply."], eligibleHistoryCount: nil, reviewHistoryCount: nil, blockedHistoryCount: nil, historyComplete: false)
    }
}
