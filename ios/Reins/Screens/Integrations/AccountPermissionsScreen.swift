import SwiftUI

/// The same canonical editor finishes a verified connection and edits it later from Connections or Trust.
struct AccountPermissionsScreen: View {
    var service: String
    var account: String
    var finishing = false
    var temporary = false
    var onDone: (() -> Void)?
    var onAnother: (() -> Void)?
    @Environment(AppModel.self) private var app
    @State private var editor: AccountPermissionsModel?
    @State private var scopeFor: PermissionCapability?
    @State private var editLimits = false

    var body: some View {
        Group {
            if let editor {
                if let preview = editor.preview { review(editor, preview) }
                else { form(editor) }
            } else { ProgressView() }
        }
        .navigationTitle(temporary ? "Temporary permission" : finishing ? "Account connected" : "Account permissions")
        .navigationBarTitleDisplayMode(.large)
        .navigationBarBackButtonHidden(editor?.preview != nil)
        .toolbar {
            if let editor, editor.preview != nil {
                ToolbarItem(placement: .topBarLeading) { Button("Back") { editor.discardPreview() }.disabled(editor.busy) }
            }
        }
        .pageBackground()
        .task(id: "\(service)/\(account)/\(temporary)/\(app.accountEpoch)") {
            let model = AccountPermissionsModel(app: app, service: service, account: account, temporary: temporary)
            editor = model
            await model.reload()
        }
        .onDisappear { editor?.deactivate() }
        .sheet(isPresented: Binding(get: { scopeFor != nil }, set: { if !$0 { scopeFor = nil; editor?.closeOptions() } })) {
            if let editor, let capability = scopeFor {
                NavigationStack { PermissionScopePicker(editor: editor, capability: capability) { scopeFor = nil; editor.closeOptions() } }
            }
        }
    }

    private func form(_ editor: AccountPermissionsModel) -> some View {
        List {
            Section {
                if finishing {
                    Label("Account connected", systemImage: "checkmark.circle.fill").textRole(.title)
                    Text(untrusted(account)).textSelection(.enabled).accessibilityIdentifier("connectedIdentity")
                    Text(editor.canEdit ? "The connection is saved. Choose permissions for selected agents below." : "The connection is saved.").foregroundStyle(Palette.ink2)
                }
                if editor.loading { ProgressView("Loading permissions…") }
                if let view = editor.view, !(temporary && editor.saved) {
                    Text(untrusted(view.scopeSummary)).accessibilityIdentifier("effectiveAccountPolicy")
                    PermissionRestrictions(restrictions: view.restrictions)
                    if !view.usageVerified { Text("Usage and remaining quota are unavailable. Automatic permissions cannot be enabled until history is verified.").font(.footnote).foregroundStyle(Palette.ink2) }
                    if editor.canEdit && (!finishing || editor.hasChosenPreset) {
                        Text("Allow automatically is a scoped, explicit permission. It is separate from Autopilot confidence. Additional review requirements and sensitive-data protections still apply.")
                            .foregroundStyle(Palette.ink2)
                    }
                }
                if let error = editor.error { Banner(untrusted(error), kind: .error).accessibilityIdentifier("permissionsError") }
                if editor.changedElsewhere {
                    Banner("Permissions changed while you were editing. Reload and review the current settings before saving.", kind: .warning)
                    ActionButton(title: "Reload current settings", busy: editor.busy, lineLimit: nil) { Task { await editor.reload() } }.accessibilityIdentifier("permissionsReload")
                }
                if editor.saved {
                    Banner(temporary ? "Temporary permission created." : "Permissions saved.").accessibilityIdentifier("permissionsSaved")
                    if let issued = editor.issued {
                        Text(untrusted(issued.scopeSummary))
                        Text(untrusted(issued.effectiveReason)).foregroundStyle(Palette.ink2)
                        Text("Expires \(PermissionCopy.expiry(issued.expiresAt))").font(.footnote)
                        PermissionRestrictions(restrictions: issued.restrictions)
                    }
                    ActionButton(title: "Done", lineLimit: nil, action: finish).accessibilityIdentifier("permissionsDone")
                }
            }
            .listRowBackground(Color.clear)
            if let view = editor.view, !editor.canEdit {
                Section("Current permissions") {
                    Text("Permission changes are not available for this account. Existing permissions and approval settings remain in effect.").accessibilityIdentifier("permissionsReadOnly")
                    ForEach(view.capabilities, id: \.capability) { capability in
                        Text(untrusted(capability.title))
                        Text(untrusted(capability.scopeSummary)).font(.footnote).foregroundStyle(Palette.ink2)
                        Text(untrusted(capability.effectiveReason)).foregroundStyle(Palette.ink2)
                        PermissionRestrictions(restrictions: capability.restrictions)
                    }
                }
            }
            if let view = editor.view, let draft = editor.draft, !editor.saved {
                if !temporary { presets(editor, view, draft) }
                if !finishing || editor.hasChosenPreset {
                    Section { Text("Review by default asks for confirmation unless a matching explicit temporary permission allows the action. Mandatory review and blocks still apply.").font(.footnote).foregroundStyle(Palette.ink2) }
                    agents(editor, view, draft)
                    if !finishing || draft.preset == .custom {
                        ForEach(view.capabilities, id: \.capability) { capability in capabilitySection(editor, capability, draft) }
                    } else {
                        Section("Your choices") {
                            ForEach(draft.rules, id: \.capability) { rule in
                                Text("\(view.capabilities.first(where: { $0.capability == rule.capability })?.title ?? "Operation"): \(PermissionCopy.mode(rule.mode))")
                            }
                        }
                    }
                    if draft.rules.contains(where: { $0.mode == .auto }) { limits(editor, view, draft) }
                    Section {
                        ActionButton(title: "Review changes", busy: editor.busy, lineLimit: nil) { Task { await editor.reviewChanges() } }
                            .disabled(editor.changedElsewhere || (!view.usageVerified && draft.rules.contains(where: { $0.mode == .auto }))).accessibilityIdentifier("permissionsPreview")
                        if !finishing, !temporary, view.usageVerified, view.capabilities.contains(where: { $0.supportedModes.contains(.auto) }) {
                            ActionButton(title: "Create a temporary permission", kind: .secondary, lineLimit: nil) { app.show(.accountPermissions(service: service, account: account, temporary: true)) }
                                .accessibilityIdentifier("accountDelegate")
                        }
                    }
                }
            }
            if finishing, !editor.saved {
                Section {
                    ActionButton(title: editor.view != nil && !editor.canEdit ? "Done" : "Later — keep current permissions", kind: .secondary, lineLimit: nil, action: finish)
                        .disabled(editor.busy).accessibilityIdentifier("connectionDone")
                    if let onAnother { ActionButton(title: "Add another account", kind: .secondary, lineLimit: nil, action: onAnother).disabled(editor.busy).accessibilityIdentifier("connectionAddAnother") }
                }
            }
        }
        .integrationList()
        .accessibilityIdentifier(finishing ? "integrationConnected" : "accountPermissionsEditor")
    }

    private func presets(_ editor: AccountPermissionsModel, _ view: AccountPermissionsView, _ draft: AccountPermissionsDraft) -> some View {
        Section("Starting point") {
            ForEach([AccountPermissionPreset.reviewEveryRequest, .readAssist, .custom], id: \.self) { preset in
                ActionButton(title: PermissionCopy.preset(preset), kind: (!finishing || editor.hasChosenPreset) && draft.preset == preset ? .primary : .secondary, busy: editor.busy, lineLimit: nil) { Task { await editor.preset(preset) } }
                    .disabled(editor.changedElsewhere || !view.availablePresets.contains(preset) || (preset == .readAssist && !view.usageVerified))
                    .accessibilityIdentifier("permissionPreset:\(preset)")
            }
        }
    }
    private func agents(_ editor: AccountPermissionsModel, _ view: AccountPermissionsView, _ draft: AccountPermissionsDraft) -> some View {
        Section {
            if view.agents.isEmpty { Text("Connect an agent first.") }
            ForEach(view.agents, id: \.connectionId) { agent in
                Toggle(isOn: Binding(get: { draft.agentIds.contains(agent.connectionId) }, set: { editor.agent(agent.connectionId, selected: $0) })) {
                    VStack(alignment: .leading, spacing: 4) {
                        Text(untrusted(agent.label))
                        Text(agent.hasAccountAccess ? "Already has account access" : "Will be allowed to discover this account").font(.footnote).foregroundStyle(Palette.ink2)
                    }
                }.disabled(editor.busy || editor.changedElsewhere).accessibilityIdentifier("permissionAgent:\(agent.connectionId)")
            }
        } header: { Text("Agents") } footer: { Text("Only selected agents are included. Selecting an agent without access explicitly admits it to this account. New agents receive no access automatically.") }
    }
    @ViewBuilder private func capabilitySection(_ editor: AccountPermissionsModel, _ capability: PermissionCapabilityView, _ draft: AccountPermissionsDraft) -> some View {
        let rule = draft.rules.first(where: { $0.capability == capability.capability })
        if !temporary || capability.supportedModes.contains(.auto) {
            Section(untrusted(capability.title)) {
                Text(untrusted(capability.description)).foregroundStyle(Palette.ink2)
                Text("Current settings: \(untrusted(capability.scopeSummary))")
                Text(untrusted(capability.effectiveReason))
                PermissionRestrictions(restrictions: capability.restrictions)
                if temporary {
                    Toggle("Allow automatically", isOn: Binding(get: { rule != nil }, set: { editor.include(capability.capability, selected: $0) }))
                        .disabled(editor.busy || editor.changedElsewhere || editor.view?.usageVerified != true)
                } else if let rule {
                    ForEach([PermissionMode.ask, .auto, .block], id: \.self) { mode in
                        ActionButton(title: PermissionCopy.mode(mode), kind: rule.mode == mode ? .primary : .secondary, lineLimit: nil) { editor.mode(capability.capability, mode) }
                            .disabled(editor.busy || editor.changedElsewhere || !capability.supportedModes.contains(mode) || (mode == .auto && editor.view?.usageVerified != true))
                            .accessibilityIdentifier("permissionMode:\(capability.capability):\(mode)")
                    }
                }
                if let rule, rule.mode == .auto {
                    Text(PermissionCopy.scope(rule.scope.kind) + (rule.scope.kind == .allInAccount ? "" : " · \(rule.scope.ids.count) selected"))
                    ActionButton(title: "Choose scope", kind: .secondary, lineLimit: nil) { scopeFor = capability.capability }.disabled(editor.busy || editor.changedElsewhere)
                        .accessibilityIdentifier("permissionScope:\(capability.capability)")
                } else if !temporary { Text("For these operations in this account, for the selected agents.").font(.footnote).foregroundStyle(Palette.ink2) }
            }
        }
    }
    private func limits(_ editor: AccountPermissionsModel, _ view: AccountPermissionsView, _ draft: AccountPermissionsDraft) -> some View {
        Section("Automatic permission limits") {
            Text("Expires: \(PermissionCopy.expiry(draft.expiresAt))")
            Text("Uses available: \(view.usageVerified ? draft.maxUses.map(String.init) ?? "Not set" : "Unavailable")")
            if draft.expiresAt == nil || draft.maxUses == nil { Text("Choose an expiry and enter a use limit before reviewing automatic permission.").font(.footnote) }
            if !editLimits, draft.expiresAt != nil, draft.maxUses != nil { ActionButton(title: "Change limits", kind: .secondary, lineLimit: nil) { editLimits = true } }
            if editLimits || draft.expiresAt == nil || draft.maxUses == nil {
                ForEach(PermissionCopy.durations.filter { $0.seconds <= (temporary ? view.delegationMaximumDurationSecs : view.maximumDurationSecs) }, id: \.seconds) { duration in
                    ActionButton(title: "Expire in \(duration.label)", kind: .secondary, lineLimit: nil) { editor.expiresAt(Int64(Date().timeIntervalSince1970) + Int64(duration.seconds)) }.disabled(editor.busy)
                }
                TextField("Remaining uses to authorize", text: Binding(get: { view.usageVerified ? draft.maxUses.map(String.init) ?? "" : "" }, set: { editor.remainingUses(UInt64($0)) }))
                    .keyboardType(.numberPad).disabled(editor.busy || !view.usageVerified).accessibilityIdentifier("permissionUses")
            }
            Text(view.usageVerified ? "Already used: \(view.used). Limits are enforced by the core; unknown outcomes are not silently refunded." : "Usage is unavailable until history is verified.").font(.footnote).foregroundStyle(Palette.ink2)
        }
    }
    private func review(_ editor: AccountPermissionsModel, _ preview: PermissionPreview) -> some View {
        List {
            Section("Review permissions") {
                Text(untrusted(preview.summary)).textSelection(.enabled).accessibilityIdentifier("permissionPreviewSummary")
                ForEach(preview.warnings, id: \.self) { Text(untrusted($0)).foregroundStyle(Palette.warning) }
                Text(PermissionCopy.history(preview)).font(.footnote).foregroundStyle(Palette.ink2)
            }
            Section {
                ActionButton(title: temporary ? "Create temporary permission" : "Save permissions", busy: editor.busy, lineLimit: nil) { Task { await editor.save() } }.accessibilityIdentifier("permissionsSave")
                ActionButton(title: "Keep editing", kind: .secondary, lineLimit: nil) { editor.discardPreview() }.disabled(editor.busy)
            }
        }.integrationList()
    }
    private func finish() { if let onDone { onDone() } else { app.back() } }
}

struct PermissionRestrictions: View {
    var restrictions: [PermissionRestriction]
    var body: some View {
        ForEach(Array(restrictions.enumerated()), id: \.offset) { _, restriction in
            Text("\(PermissionCopy.controller(restriction.controller)): \(untrusted(restriction.reason))").font(.footnote).foregroundStyle(Palette.ink2)
        }
    }
}

enum PermissionCopy {
    static func mode(_ mode: PermissionMode) -> String { switch mode { case .ask: "Review by default"; case .auto: "Allow automatically"; case .block: "Block" } }
    static func preset(_ preset: AccountPermissionPreset) -> String { switch preset { case .reviewEveryRequest: "Review by default"; case .readAssist: "Allow searching and reading"; case .custom: "Customize" } }
    static func scope(_ scope: PermissionScopeKind) -> String { switch scope { case .allInAccount: "All in this account"; case .selectedRecipients: "Selected recipients"; case .selectedResources: "Selected messages" } }
    static func controller(_ value: PermissionController) -> String { switch value { case .owner: "Your policy"; case .organization: "Organization policy"; case .unsupportedAdapter: "Integration limitation"; case .conflict: "Policy conflict" } }
    static func expiry(_ at: Int64?) -> String { at.map { Date(timeIntervalSince1970: TimeInterval($0)).formatted(date: .abbreviated, time: .shortened) } ?? "Choose an expiry" }
    static let durations: [(seconds: UInt64, label: String)] = [(3600, "1 hour"), (28800, "8 hours"), (86400, "1 day"), (604800, "7 days")]
    static func history(_ preview: PermissionPreview) -> String {
        "Retained history: automatic \(preview.eligibleHistoryCount.map(String.init) ?? "unavailable"), review \(preview.reviewHistoryCount.map(String.init) ?? "unavailable"), blocked \(preview.blockedHistoryCount.map(String.init) ?? "unavailable")" + (preview.historyComplete ? "" : "\nHistory coverage is partial. These counts are not a prediction of future requests.")
    }
}

private struct PermissionScopePicker: View {
    var editor: AccountPermissionsModel
    var capability: PermissionCapability
    var onDone: () -> Void
    @State private var query = ""
    private var selected: PermissionScopeSelection? { editor.draft?.rules.first(where: { $0.capability == capability })?.scope }
    var body: some View {
        List {
            if editor.optionsLoading { ProgressView("Loading scope choices…") }
            if let options = editor.options {
                if let reason = options.unsupportedReason { Text(untrusted(reason)).foregroundStyle(Palette.warning) }
                ForEach(options.kinds, id: \.self) { kind in
                    ActionButton(title: PermissionCopy.scope(kind), kind: selected?.kind == kind ? .primary : .secondary, lineLimit: nil) { if selected?.kind != kind { editor.scope(capability, PermissionScopeSelection(kind: kind, ids: [])) } }
                        .disabled(editor.busy || editor.changedElsewhere)
                }
                if let selected, selected.kind != .allInAccount {
                    TextField(options.allowsAddressEntry ? "Search or enter an email address" : "Search messages", text: $query)
                        .textInputAutocapitalization(.never).autocorrectionDisabled().accessibilityIdentifier("permissionScopeQuery")
                    ForEach(options.options.filter { $0.service == editor.service && $0.account == editor.account && $0.kind == selected.kind }, id: \.id) { option in
                        Toggle(isOn: Binding(get: { selected.ids.contains(option.id) }, set: { checked in
                            var ids = selected.ids.filter { $0 != option.id }; if checked { ids.append(option.id) }
                            editor.scope(capability, PermissionScopeSelection(kind: selected.kind, ids: ids))
                        })) {
                            VStack(alignment: .leading) { Text(untrusted(option.label)); Text(untrusted(option.detail)).font(.footnote).foregroundStyle(Palette.ink2) }
                        }.disabled(editor.optionsLoading || editor.busy || editor.changedElsewhere).accessibilityIdentifier("scopeOption:\(option.id)")
                    }
                }
            }
            if let error = editor.error { Banner(untrusted(error), kind: .error) }
        }
        .integrationList().navigationTitle("Permission scope")
        .toolbar { ToolbarItem(placement: .confirmationAction) { Button("Done", action: onDone) } }
        .task(id: query) { await editor.scopeOptions(capability, query: query) }
    }
}
