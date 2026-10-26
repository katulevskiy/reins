import SwiftUI

/// The same canonical editor finishes a verified connection and edits it later from Connections or Trust (the
/// Permissions board, 02 to 04; quiet): where to start from, which agents, each operation's rule (on its own Rules
/// page), then a review of exactly what will be saved, saved with the owner's check.
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
    @State private var rulesOpen = false

    var body: some View {
        Group {
            if let editor {
                if let preview = editor.preview { review(editor, preview) }
                else { form(editor) }
            } else { ProgressView() }
        }
        .navigationTitle("")
        .navigationBarTitleDisplayMode(.inline)
        .navigationBarBackButtonHidden(editor?.preview != nil)
        .toolbar {
            if let editor, editor.preview != nil {
                ToolbarItem(placement: .topBarLeading) { Button("Back") { editor.discardPreview() }.disabled(editor.busy) }
            }
        }
        .background(Palette.paper.ignoresSafeArea())
        .task(id: "\(service)/\(account)/\(temporary)/\(app.accountEpoch)") {
            editor?.deactivate()
            let model = AccountPermissionsModel(app: app, service: service, account: account, temporary: temporary)
            editor = model
            await model.reload()
            #if DEBUG
            // `-openRules` / `-openReview` (screenshots): the Rules page, or the review of the current draft.
            let args = ProcessInfo.processInfo.arguments
            if args.contains("-readAssist") { await model.preset(.readAssist) }
            if args.contains("-openRules") { rulesOpen = true }
            if args.contains("-openReview") { await model.reviewChanges() }
            #endif
        }
        .onDisappear { if !rulesOpen { editor?.deactivate() } }
        .sheet(isPresented: Binding(get: { scopeFor != nil }, set: { if !$0 { scopeFor = nil; editor?.closeOptions() } })) {
            if let editor, let capability = scopeFor {
                NavigationStack { PermissionScopePicker(editor: editor, capability: capability) { scopeFor = nil; editor.closeOptions() } }
            }
        }
        .navigationDestination(isPresented: $rulesOpen) {
            if let editor {
                rules(editor)
                    .navigationTitle(untrusted(account))
                    .navigationBarTitleDisplayMode(.inline)
            }
        }
    }

    // MARK: The account (02)

    private func form(_ editor: AccountPermissionsModel) -> some View {
        List {
            header(editor)
            if let view = editor.view, view.service == "gmail", !view.capabilities.contains(where: { $0.supportedModes.contains(.auto) }) {
                PrimaryPhoneSetupSection(editor: editor)
            }
            if let view = editor.view, !editor.canEdit {
                QuietHeader(title: "Current permissions").quietRow(sides: 0)
                VStack(alignment: .leading, spacing: 10) {
                    QuietLine("Permission changes are not available for this account. Existing permissions and approval settings remain in effect.")
                        .accessibilityIdentifier("permissionsReadOnly")
                    ForEach(view.capabilities, id: \.capability) { capability in
                        VStack(alignment: .leading, spacing: 4) {
                            Text(untrusted(capability.title)).textRole(.row).foregroundStyle(Palette.ink)
                            QuietLine(untrusted(capability.scopeSummary))
                            QuietLine(untrusted(capability.effectiveReason))
                            PermissionRestrictions(restrictions: capability.restrictions)
                        }
                    }
                }
                .quietRow()
            }
            if let view = editor.view, let draft = editor.draft, !editor.saved {
                if !temporary { presets(editor, view, draft) }
                if !finishing || editor.hasChosenPreset {
                    agents(editor, view, draft)
                    if temporary {
                        capabilities(editor, view, draft)
                    } else {
                        now(editor, view, draft, opens: !finishing || draft.preset == .custom)
                    }
                }
            }
            if finishing, !editor.saved {
                VStack(spacing: 10) {
                    Button(editor.view != nil && !editor.canEdit ? "Done" : "Later — keep current permissions", action: finish)
                        .buttonStyle(SignalButtonLook(kind: .soft))
                        .frame(height: 54)
                        .disabled(editor.busy).accessibilityIdentifier("connectionDone")
                    if let onAnother {
                        Button("Add another account", action: onAnother)
                            .buttonStyle(SignalButtonLook(kind: .outline))
                            .frame(height: 54)
                            .disabled(editor.busy).accessibilityIdentifier("connectionAddAnother")
                    }
                }
                .quietRow(top: 20, sides: 16)
            }
        }
        .integrationList()
        .accessibilityIdentifier(finishing ? "integrationConnected" : "accountPermissionsEditor")
        .safeAreaInset(edge: .bottom, spacing: 0) { foot(editor) }
    }

    /// The title (the account, or "Account connected" while finishing) and every state line under it.
    @ViewBuilder private func header(_ editor: AccountPermissionsModel) -> some View {
        Group {
            if finishing {
                QuietTitle(title: "Account connected") {
                    GlyphIcon(.circleCheck, size: 24).foregroundStyle(Palette.success)
                } trail: { EmptyView() }
            } else if temporary {
                QuietTitle(title: "Temporary") { ServiceAvatar(service: service, size: 32) } trail: { EmptyView() }
            } else {
                QuietTitle(title: untrusted(account)) { ServiceAvatar(service: service, size: 32) } trail: { EmptyView() }
            }
        }
        .quietRow(sides: 0)
        VStack(alignment: .leading, spacing: 8) {
            if finishing || temporary {
                Text(untrusted(account)).font(RFont.mono(14.5)).foregroundStyle(Palette.ink).textSelection(.enabled)
                    .accessibilityIdentifier("connectedIdentity")
            }
            if finishing {
                QuietLine(editor.canEdit ? "The connection is saved. Choose permissions for selected agents below." : "The connection is saved.")
            }
            if editor.loading { ProgressView("Loading permissions…") }
            if let view = editor.view, !(temporary && editor.saved) {
                QuietLine(untrusted(view.scopeSummary)).accessibilityIdentifier("effectiveAccountPolicy")
                PermissionRestrictions(restrictions: view.restrictions)
                if !view.usageVerified { WarningLine("Usage and remaining quota are unavailable. Automatic permissions cannot be enabled until history is verified.") }
            }
            if let error = editor.error { Banner(untrusted(error), kind: .error).accessibilityIdentifier("permissionsError") }
            if editor.changedElsewhere {
                Banner("Permissions changed while you were editing. Reload and review the current settings before saving.", kind: .warning)
                Button("Reload current settings") { Task { await editor.reload() } }
                    .buttonStyle(SignalButtonLook(kind: .soft, size: .small))
                    .disabled(editor.busy)
                    .accessibilityIdentifier("permissionsReload")
            }
            if editor.saved {
                Banner(temporary ? "Temporary permission created." : "Permissions saved.").accessibilityIdentifier("permissionsSaved")
                if let issued = editor.issued {
                    QuietLine(untrusted(issued.scopeSummary))
                    QuietLine(untrusted(issued.effectiveReason))
                    QuietLine("Expires \(PermissionCopy.expiry(issued.expiresAt))")
                    PermissionRestrictions(restrictions: issued.restrictions)
                }
                Button("Done", action: finish)
                    .buttonStyle(SignalButtonLook(kind: .inverse))
                    .frame(height: 54)
                    .accessibilityIdentifier("permissionsDone")
            }
        }
        .quietRow(top: 8)
    }

    /// Start from: three choices as radio rows.
    @ViewBuilder private func presets(_ editor: AccountPermissionsModel, _ view: AccountPermissionsView, _ draft: AccountPermissionsDraft) -> some View {
        QuietHeader(
            title: "Start from",
            help: editor.canEdit ? "Allow automatically is a scoped, explicit permission, separate from Autopilot. Review requirements and sensitive-data protections still apply." : nil
        )
        .quietRow(sides: 0)
        FlatGroup(inset: 44) {
            ForEach([AccountPermissionPreset.reviewEveryRequest, .readAssist, .custom], id: \.self) { preset in
                let on = (!finishing || editor.hasChosenPreset) && draft.preset == preset
                Button { Task { await editor.preset(preset) } } label: {
                    FlatRow(title: PermissionCopy.preset(preset)) { GlyphTile(PermissionLook.glyph(preset)) } trail: { RadioDot(selected: on) }
                }
                .buttonStyle(PressDimsStyle())
                .disabled(editor.busy || editor.changedElsewhere || !view.availablePresets.contains(preset) || (preset == .readAssist && !view.usageVerified))
                .accessibilityLabel(PermissionCopy.preset(preset))
                .accessibilityAddTraits(on ? .isSelected : [])
                .accessibilityIdentifier("permissionPreset:\(preset)")
            }
        }
        .quietRow(sides: 0)
    }

    /// Agents: a switch each; selecting one without access admits it to this account.
    @ViewBuilder private func agents(_ editor: AccountPermissionsModel, _ view: AccountPermissionsView, _ draft: AccountPermissionsDraft) -> some View {
        QuietHeader(
            title: "Agents",
            help: "Automatic permissions require selected agents. With no selection, review and block rules apply to all agents, including future connections. Selecting an agent without access explicitly admits it to this account.",
            count: draft.agentIds.isEmpty ? nil : "\(draft.agentIds.count)"
        )
        .quietRow(sides: 0)
        FlatGroup(inset: 44) {
            if view.agents.isEmpty {
                QuietLine("No agents are connected. You can still set review or block defaults.").frame(minHeight: 52)
            }
            ForEach(view.agents, id: \.connectionId) { agent in
                Toggle(isOn: Binding(get: { draft.agentIds.contains(agent.connectionId) }, set: { editor.agent(agent.connectionId, selected: $0) })) {
                    HStack(spacing: 12) {
                        ConnectionIcon(connectionId: agent.connectionId, label: agent.label, size: 32)
                        VStack(alignment: .leading, spacing: 1) {
                            Text(verbatim: untrustedName(agent.label)).textRole(.row).foregroundStyle(Palette.ink).lineLimit(1)
                            Text(agent.hasAccountAccess ? "Already has account access" : "Will be allowed to discover this account")
                                .textRole(.sub).foregroundStyle(Palette.inkMeta)
                        }
                    }
                }
                .toggleStyle(SignalSwitchStyle())
                .frame(minHeight: 56)
                .disabled(editor.busy || editor.changedElsewhere)
                .accessibilityIdentifier("permissionAgent:\(agent.connectionId)")
            }
        }
        .quietRow(sides: 0)
        if draft.agentIds.isEmpty {
            QuietLine("All agents (including future connections) for review and block rules. Automatic permission requires selected agents.")
                .accessibilityIdentifier("permissionAllAgents")
                .quietRow(top: 8)
        }
    }

    /// Now: each operation with its rule; they open the Rules page.
    @ViewBuilder private func now(_ editor: AccountPermissionsModel, _ view: AccountPermissionsView, _ draft: AccountPermissionsDraft, opens: Bool) -> some View {
        QuietHeader(
            title: opens ? "Now" : "Your choices",
            help: "Review by default asks for confirmation unless a matching explicit temporary permission allows the action. Mandatory review and blocks still apply."
        )
        .quietRow(sides: 0)
        FlatGroup(inset: 44) {
            ForEach(view.capabilities, id: \.capability) { capability in
                let mode = draft.rules.first(where: { $0.capability == capability.capability })?.mode
                Button { rulesOpen = true } label: {
                    FlatRow(title: untrusted(capability.title), value: mode.map(PermissionLook.word), chevron: opens) {
                        GlyphTile(PermissionLook.glyph(capability.capability))
                    }
                }
                .buttonStyle(PressDimsStyle())
                .disabled(!opens)
                .accessibilityIdentifier("permissionRule:\(capability.capability)")
            }
        }
        .quietRow(sides: 0)
    }

    /// Review, and (for an account with automatic operations) a temporary permission.
    @ViewBuilder private func foot(_ editor: AccountPermissionsModel) -> some View {
        if let view = editor.view, let draft = editor.draft, !editor.saved, !finishing || editor.hasChosenPreset {
            FootButtons {
                if !finishing, !temporary, view.usageVerified, view.capabilities.contains(where: { $0.supportedModes.contains(.auto) }) {
                    Button { app.show(.accountPermissions(service: service, account: account, temporary: true)) } label: { GlyphLabel("Temporary", glyph: .timer) }
                        .buttonStyle(SignalButtonLook(kind: .soft))
                        .frame(maxWidth: .infinity)
                        .accessibilityIdentifier("accountDelegate")
                }
                Button { Task { await editor.reviewChanges() } } label: {
                    ZStack {
                        Text("Review").opacity(editor.busy ? 0 : 1)
                        if editor.busy { ProgressView() }
                    }
                }
                .buttonStyle(SignalButtonLook(kind: .inverse))
                .frame(maxWidth: .infinity)
                .disabled(editor.changedElsewhere || (!view.usageVerified && draft.rules.contains(where: { $0.mode == .auto })))
                .accessibilityIdentifier("permissionsPreview")
            }
        }
    }

    // MARK: Rules (03)

    private func rules(_ editor: AccountPermissionsModel) -> some View {
        List {
            QuietTitle("Rules").quietRow(sides: 0)
            if let error = editor.error { Banner(untrusted(error), kind: .error).quietRow(top: 8) }
            if let view = editor.view, let draft = editor.draft {
                capabilities(editor, view, draft)
            }
        }
        .integrationList()
        .background(Palette.paper.ignoresSafeArea())
        .safeAreaInset(edge: .bottom, spacing: 0) { foot(editor) }
        .accessibilityIdentifier("permissionRules")
    }

    /// Each operation: Review, Allow or Block (a switch, for a temporary permission); allowed, its scope and the limits.
    @ViewBuilder private func capabilities(_ editor: AccountPermissionsModel, _ view: AccountPermissionsView, _ draft: AccountPermissionsDraft) -> some View {
        let shown = view.capabilities.filter { !temporary || $0.supportedModes.contains(.auto) }
        let firstAuto = shown.first { c in draft.rules.contains { $0.capability == c.capability && $0.mode == .auto } }?.capability
        ForEach(shown, id: \.capability) { capability in
            let rule = draft.rules.first(where: { $0.capability == capability.capability })
            QuietHeader(title: untrusted(capability.title), glyph: PermissionLook.glyph(capability.capability), help: untrusted(capability.description))
                .quietRow(sides: 0)
            VStack(alignment: .leading, spacing: 8) {
                if temporary {
                    Toggle(isOn: Binding(get: { rule != nil }, set: { editor.include(capability.capability, selected: $0) })) {
                        Text("Allow automatically").textRole(.row).foregroundStyle(Palette.ink)
                    }
                    .toggleStyle(SignalSwitchStyle())
                    .frame(minHeight: 44)
                    .disabled(editor.busy || editor.changedElsewhere || editor.view?.usageVerified != true)
                } else if let rule {
                    Chooser([PermissionMode.ask, .auto, .block].map { mode in
                        Chooser.Segment(
                            id: "permissionMode:\(capability.capability):\(mode)", title: PermissionLook.word(mode), selected: rule.mode == mode,
                            enabled: !(editor.busy || editor.changedElsewhere || !capability.supportedModes.contains(mode) || (mode == .auto && editor.view?.usageVerified != true))
                        ) { editor.mode(capability.capability, mode) }
                    })
                }
                QuietLine("Current settings: \(untrusted(capability.scopeSummary))")
                if !capability.effectiveReason.isEmpty { QuietLine(untrusted(capability.effectiveReason)) }
                PermissionRestrictions(restrictions: capability.restrictions)
            }
            .quietRow(top: 2)
            if let rule, rule.mode == .auto {
                FlatGroup(inset: 44) {
                    Button { scopeFor = capability.capability } label: {
                        FlatRow(
                            title: PermissionCopy.scope(rule.scope.kind),
                            value: rule.scope.kind == .allInAccount ? nil : "\(rule.scope.ids.count) selected", chevron: true
                        ) { GlyphTile(.folderSearch) }
                    }
                    .buttonStyle(PressDimsStyle())
                    .disabled(editor.busy || editor.changedElsewhere)
                    .accessibilityIdentifier("permissionScope:\(capability.capability)")
                    if capability.capability == firstAuto { limitRows(editor, view, draft) }
                }
                .quietRow(top: 8, sides: 0)
                if capability.capability == firstAuto { limitNotes(view, draft) }
            }
        }
    }

    /// Expires (a menu of lengths) and Uses (typed), shared by every automatic operation.
    @ViewBuilder private func limitRows(_ editor: AccountPermissionsModel, _ view: AccountPermissionsView, _ draft: AccountPermissionsDraft) -> some View {
        let durations = PermissionCopy.durations.filter { $0.seconds <= (temporary ? view.delegationMaximumDurationSecs : view.maximumDurationSecs) }
        Menu {
            ForEach(durations, id: \.seconds) { duration in
                Button(duration.label) { editor.expiresAt(Int64(Date().timeIntervalSince1970) + Int64(duration.seconds)) }
            }
        } label: {
            FlatRow(title: "Expires", value: draft.expiresAt.map { PermissionCopy.expiry($0) } ?? "Choose", chevron: true) { GlyphTile(.timer) }
        }
        .disabled(editor.busy)
        .accessibilityIdentifier("permissionExpires")
        FlatRow(title: temporary ? "Total uses" : "Uses") { GlyphTile(.card) } trail: {
            TextField("20", text: Binding(get: { view.usageVerified ? draft.maxUses.map(String.init) ?? "" : "" }, set: { editor.remainingUses(UInt64($0)) }))
                .keyboardType(.numberPad)
                .multilineTextAlignment(.trailing)
                .font(RFont.sans(15))
                .foregroundStyle(Palette.ink2)
                .frame(maxWidth: 90)
                .disabled(editor.busy || !view.usageVerified)
                .accessibilityLabel(temporary ? "Total uses to authorize" : "Further uses per automatic operation")
                .accessibilityIdentifier("permissionUses")
        }
    }

    @ViewBuilder private func limitNotes(_ view: AccountPermissionsView, _ draft: AccountPermissionsDraft) -> some View {
        VStack(alignment: .leading, spacing: 6) {
            if draft.expiresAt == nil || draft.maxUses == nil { WarningLine("Choose an expiry and enter a use limit before reviewing automatic permission.") }
            if view.usageVerified {
                QuietLine("Already used: \(view.used). Limits are enforced by the core; unknown outcomes are not silently refunded.")
            } else {
                WarningLine("Usage is unavailable until history is verified.")
            }
        }
        .quietRow(top: 8)
    }

    // MARK: Review (04)

    private func review(_ editor: AccountPermissionsModel, _ preview: PermissionPreview) -> some View {
        List {
            QuietTitle("Review").quietRow(sides: 0)
            Text(untrusted(preview.summary))
                .textRole(.sub)
                .foregroundStyle(Palette.ink2)
                .textSelection(.enabled)
                .accessibilityIdentifier("permissionPreviewSummary")
                .quietRow(top: 6)
            if let view = editor.view, let draft = editor.draft { PermissionDraftReview(view: view, draft: draft, temporary: temporary) }
            VStack(alignment: .leading, spacing: 8) {
                ForEach(preview.warnings, id: \.self) { WarningLine(untrusted($0)) }
                QuietLine(PermissionCopy.history(preview)).accessibilityIdentifier("permissionPreviewHistory")
            }
            .quietRow(top: 12)
        }
        .integrationList()
        .safeAreaInset(edge: .bottom, spacing: 0) {
            FootButtons {
                Button("Edit") { editor.discardPreview() }
                    .buttonStyle(SignalButtonLook(kind: .outline))
                    .frame(maxWidth: .infinity)
                    .disabled(editor.busy)
                Button { Task { await editor.save() } } label: {
                    ZStack {
                        GlyphLabel(temporary ? "Create" : "Save", glyph: .faceId).opacity(editor.busy ? 0 : 1)
                        if editor.busy { ProgressView() }
                    }
                }
                .buttonStyle(SignalButtonLook(kind: .inverse))
                .frame(maxWidth: .infinity)
                .accessibilityIdentifier("permissionsSave")
            }
        }
    }
    private func finish() { if let onDone { onDone() } else { app.back() } }
}

struct PermissionRestrictions: View {
    var restrictions: [PermissionRestriction]
    var body: some View {
        ForEach(Array(restrictions.enumerated()), id: \.offset) { _, restriction in
            WarningLine("\(PermissionCopy.controller(restriction.controller)): \(untrusted(restriction.reason))")
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
        if preview.eligibleHistoryCount == nil, preview.reviewHistoryCount == nil, preview.blockedHistoryCount == nil { return "A history comparison is unavailable." }
        return "Past requests: \(preview.eligibleHistoryCount.map(String.init) ?? "unavailable") automatic, \(preview.reviewHistoryCount.map(String.init) ?? "unavailable") review, \(preview.blockedHistoryCount.map(String.init) ?? "unavailable") blocked." + (preview.historyComplete ? "" : " Only part of your history is available. Past results do not predict future requests.")
    }

}

/// Displays only the exact typed draft already validated by the core preview: the account and agents, account access,
/// each operation that changes (allowed ones with their scope and limits), then the ones that stay as they are.
private struct PermissionDraftReview: View {
    var view: AccountPermissionsView
    var draft: AccountPermissionsDraft
    var temporary: Bool
    private func label(_ id: String) -> String { view.agents.first(where: { $0.connectionId == id })?.label ?? id }
    private var admissions: [String] { draft.agentIds.filter { id in !view.agents.contains(where: { $0.connectionId == id && $0.hasAccountAccess }) } }
    private func capability(_ c: PermissionCapability) -> PermissionCapabilityView? { view.capabilities.first { $0.capability == c } }
    private func changed(_ rule: CapabilityPermissionInput) -> Bool {
        temporary || capability(rule.capability)?.configuredMode != rule.mode || rule.mode == .auto
    }

    var body: some View {
        FlatGroup(inset: 44) {
            FlatRow(title: untrusted(draft.account), mono: true) { ServiceAvatar(service: draft.service, size: 32) }
                .textSelection(.enabled)
                .accessibilityLabel("\(serviceName(draft.service)) · \(untrusted(draft.account))")
            if draft.agentIds.isEmpty {
                FlatRow(title: "All agents (including future connections)") { GlyphTile(.person) }
            }
            ForEach(draft.agentIds, id: \.self) { id in
                HStack(spacing: 12) {
                    ConnectionIcon(connectionId: id, label: label(id), size: 32)
                    VStack(alignment: .leading, spacing: 1) {
                        Text(verbatim: untrustedName(label(id))).textRole(.row).foregroundStyle(Palette.ink).lineLimit(1)
                        Text(verbatim: untrusted(id)).font(RFont.mono(12)).foregroundStyle(Palette.inkMeta).lineLimit(1)
                    }
                }
                .frame(maxWidth: .infinity, minHeight: 56, alignment: .leading)
            }
        }
        .quietRow(top: 16, sides: 0)
        QuietHeader(title: "Account access").quietRow(sides: 0)
        VStack(alignment: .leading, spacing: 4) {
            if draft.grantAccountAccess, !admissions.isEmpty {
                ForEach(admissions, id: \.self) { id in QuietLine("\(untrustedName(label(id))) can discover this account.") }
                QuietLine("Account access expires: \(draft.expiresAt.map { PermissionCopy.expiry($0) } ?? "No expiry")")
            } else {
                QuietLine("No additional agents receive account access.")
            }
        }
        .quietRow()
        ForEach(draft.rules.filter(changed), id: \.capability) { rule in
            QuietHeader(title: untrusted(capability(rule.capability)?.title ?? "Operation"), glyph: PermissionLook.glyph(rule.capability)).quietRow(sides: 0)
            FlatGroup(inset: 44) {
                FlatRow(title: PermissionLook.word(rule.mode), titleColor: rule.mode == .auto ? Palette.success : rule.mode == .block ? Palette.danger : Palette.ink) {
                    ModeMarkTile(mode: rule.mode)
                }
                .accessibilityLabel(PermissionCopy.mode(rule.mode))
                if rule.mode == .auto {
                    FlatRow(title: PermissionCopy.scope(rule.scope.kind)) { GlyphTile(.folderSearch) }
                    ForEach(rule.scope.ids, id: \.self) { id in
                        FlatRow(title: untrusted(id), mono: true) { GlyphTile(.mail) }.textSelection(.enabled)
                    }
                    FlatRow(title: Self.limits(draft)) { GlyphTile(.timer) }
                }
            }
            .quietRow(sides: 0)
            Group {
                if rule.mode == .auto {
                    if rule.scope.kind == .selectedRecipients { QuietLine("Every To and Cc address must match these recipients.") }
                } else {
                    QuietLine(draft.agentIds.isEmpty ? "All requests of this operation in this account, from all agents including future connections." : "All requests of this operation in this account, from the selected agents.")
                }
            }
            .quietRow(top: 6)
        }
        if draft.rules.contains(where: { $0.mode == .auto }) {
            VStack(alignment: .leading, spacing: 4) {
                QuietLine("Automatic permission expires: \(PermissionCopy.expiry(draft.expiresAt))")
                QuietLine(temporary ? "At most \(draft.maxUses.map(String.init) ?? "not set") total requests across the selected operations." : "At most \(draft.maxUses.map(String.init) ?? "not set") further uses for each automatic operation.")
            }
            .accessibilityIdentifier("permissionReviewLimits")
            .quietRow(top: 10)
        }
        let unchanged = draft.rules.filter { !changed($0) }
        if !unchanged.isEmpty {
            QuietHeader(title: "Unchanged").quietRow(sides: 0)
            FlatGroup(inset: 44) {
                ForEach(unchanged, id: \.capability) { rule in
                    FlatRow(title: untrusted(capability(rule.capability)?.title ?? "Operation"), value: PermissionLook.word(rule.mode)) {
                        GlyphTile(PermissionLook.glyph(rule.capability))
                    }
                }
            }
            .quietRow(sides: 0)
        }
    }

    /// "1 day · 20 uses": the automatic permission's limits in one line.
    static func limits(_ draft: AccountPermissionsDraft) -> String {
        let ends = draft.expiresAt.map { PermissionCopy.expiry($0) } ?? "No expiry"
        let uses = draft.maxUses.map { "\($0) uses" } ?? "no use limit"
        return "\(ends) · \(uses)"
    }
}

/// A rule's mark on its tile: Allow's check on green, Block's cross on red, Review's person on paper.
private struct ModeMarkTile: View {
    var mode: PermissionMode

    var body: some View {
        let tint: Color? = mode == .auto ? Palette.success : mode == .block ? Palette.danger : nil
        RoundedRectangle(cornerRadius: 9, style: .continuous)
            .fill(tint.map { $0.opacity(0.13) } ?? Palette.paperSunken)
            .frame(width: 32, height: 32)
            .overlay { GlyphIcon(PermissionLook.glyph(mode), size: 17).foregroundStyle(tint ?? Palette.ink2) }
            .accessibilityHidden(true)
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
