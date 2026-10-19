package dev.reins.android.ui.permissions

import androidx.activity.compose.BackHandler
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import dev.reins.android.design.Banner
import dev.reins.android.design.BannerKind
import dev.reins.android.design.ButtonStyle
import dev.reins.android.design.CapsuleButton
import dev.reins.android.design.CheckRow
import dev.reins.android.design.Group
import dev.reins.android.design.Hairline
import dev.reins.android.design.LocalColors
import dev.reins.android.design.RText
import dev.reins.android.design.RTextField
import dev.reins.android.design.RType
import dev.reins.android.design.Screen
import dev.reins.android.platform.Authenticator
import dev.reins.android.ui.common.untrusted
import dev.reins.core.AccountPermissionPreset
import dev.reins.core.PermissionCapability
import dev.reins.core.PermissionMode
import dev.reins.core.PermissionPreview
import dev.reins.core.PermissionRestriction
import dev.reins.core.PermissionScopeKind
import dev.reins.core.PermissionScopeSelection

@Composable
fun AccountPermissionsScreen(
    model: AccountPermissionsViewModel,
    authenticator: Authenticator,
    finishing: Boolean,
    onBack: () -> Unit,
    onDone: () -> Unit,
    onDelegate: () -> Unit,
    onAnother: (() -> Unit)? = null,
) {
    val state by model.ui.collectAsStateWithLifecycle()
    DisposableEffect(model) { model.activate(); onDispose { model.deactivate() } }
    LaunchedEffect(model) { model.reload() }
    val c = LocalColors.current
    var scopeFor by remember { mutableStateOf<PermissionCapability?>(null) }
    var editLimits by remember { mutableStateOf(!finishing) }
    scopeFor?.let { capability ->
        PermissionScopePicker(model, capability) { scopeFor = null; model.closeOptions() }
        return
    }
    state.preview?.let { preview ->
        BackHandler { if (!state.busy) model.discardPreview() }
        Screen(title = "Review permissions", subtitle = untrusted(model.account), onBack = { if (!state.busy) model.discardPreview() }) {
            Column(Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(16.dp)) {
                RText(permissionPreviewText(preview), RType.sans(15f, lineHeight = 23f), c.ink, Modifier.testTag("permissionPreviewSummary"))
                CapsuleButton(if (model.temporary) "Create temporary permission" else "Save permissions", Modifier.fillMaxWidth().testTag("permissionsSave"), enabled = !state.busy, busy = state.busy, maxLines = Int.MAX_VALUE) { model.save(authenticator) }
                CapsuleButton("Keep editing", Modifier.fillMaxWidth(), enabled = !state.busy, style = ButtonStyle.Soft, onClick = model::discardPreview, maxLines = Int.MAX_VALUE)
            }
        }
        return
    }
    Screen(title = if (model.temporary) "Temporary permission" else if (finishing) "Account connected" else "Account permissions", subtitle = untrusted(model.account), onBack = onBack) {
        Column(Modifier.padding(16.dp).then(if (finishing) Modifier.testTag("integrationConnected") else Modifier), verticalArrangement = Arrangement.spacedBy(12.dp)) {
            if (finishing) {
                RText(untrusted(model.account), RType.serif(25f), c.ink, Modifier.testTag("connectedIdentity"), ltr = true)
                RText(if (model.canEdit) "The connection is saved. Choose permissions for the agents you select below." else "The connection is saved.", RType.sans(15f), c.ink2)
            }
            if (state.loading) RText("Loading permissions…", RType.sans(15f), c.ink2)
            state.account?.takeUnless { model.temporary && state.saved }?.let { view ->
                RText(untrusted(view.scopeSummary), RType.sans(15f), c.ink, Modifier.testTag("effectiveAccountPolicy"))
                if (!finishing) view.capabilities.forEach { capability ->
                    capability.configuredMode?.let { RText("${untrusted(capability.title)}: ${permissionModeTitle(it)}", RType.sans(14f), c.ink) }
                }
                PermissionRestrictions(view.restrictions)
                if (!view.usageVerified) RText("Usage and remaining quota are unavailable. Automatic permissions cannot be enabled until history is verified.", RType.sans(14f), c.ink2)
                if (model.canEdit && (!finishing || state.hasChosenPreset)) RText("Allow automatically is a scoped, explicit permission. It is separate from Autopilot confidence. Additional review requirements and sensitive-data protections still apply.", RType.sans(14f), c.ink2)
            }
            state.error?.let { Banner(untrusted(it), kind = BannerKind.Error, tag = "permissionsError") }
            if (state.changedElsewhere) {
                Banner("Permissions changed while you were editing. Reload and review the current settings before saving.", kind = BannerKind.Warning, tag = "permissionsChanged")
                CapsuleButton("Reload current settings", Modifier.fillMaxWidth().testTag("permissionsReload"), enabled = !state.busy, onClick = model::reload, maxLines = Int.MAX_VALUE)
            }
            if (state.saved) {
                Banner(if (model.temporary) "Temporary permission created." else "Permissions saved.", tag = "permissionsSaved")
                state.issued?.let { issued ->
                    RText(untrusted(issued.scopeSummary), RType.sans(15f), c.ink)
                    RText(untrusted(issued.effectiveReason), RType.sans(14f), c.ink2)
                    RText("${delegationStateTitle(issued.state)} · expires ${permissionExpiry(issued.expiresAt)}", RType.sans(14f), c.ink2)
                    PermissionRestrictions(issued.restrictions)
                }
                CapsuleButton("Done", Modifier.fillMaxWidth().testTag("permissionsDone"), onClick = onDone, maxLines = Int.MAX_VALUE)
            }
        }
        val view = state.account
        if (view != null && !model.canEdit) Group(header = "Current permissions") {
            Column(Modifier.padding(16.dp).testTag("permissionsReadOnly"), verticalArrangement = Arrangement.spacedBy(12.dp)) {
                RText("Permission changes are not available for this account. Existing permissions and approval settings remain in effect.", RType.sans(15f), c.ink)
                view.capabilities.forEach { capability ->
                    RText(untrusted(capability.title), RType.sans(16f), c.ink)
                    RText(untrusted(capability.scopeSummary), RType.sans(14f), c.ink2)
                    RText(untrusted(capability.effectiveReason), RType.sans(14f), c.ink2)
                    PermissionRestrictions(capability.restrictions)
                }
            }
        }
        val draft = state.draft
        if (view != null && draft != null && !state.saved) {
            RText("Review and save to apply your choices.", RType.sans(14f), c.ink2, Modifier.padding(16.dp))
            if (!model.temporary) Group(header = "Starting point") {
                Column(Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(10.dp)) {
                    AccountPermissionPreset.entries.forEach { preset ->
                        CapsuleButton(permissionPresetTitle(preset), Modifier.fillMaxWidth().testTag("permissionPreset:$preset"),
                            enabled = !state.busy && !state.changedElsewhere && preset in view.availablePresets && (preset != AccountPermissionPreset.READ_ASSIST || view.usageVerified),
                            style = if ((!finishing || state.hasChosenPreset) && draft.preset == preset) ButtonStyle.Inverse else ButtonStyle.Soft,
                         maxLines = Int.MAX_VALUE) { model.preset(preset) }
                    }
                }
            }
            if (!finishing || state.hasChosenPreset) {
            RText("Review by default asks for confirmation unless a matching explicit temporary permission allows the action. Mandatory review and blocks still apply.", RType.sans(14f), c.ink2, Modifier.padding(horizontal = 16.dp))
            Group(header = "Agents", footer = "Only selected agents are included. Selecting an agent without access explicitly admits it to this account. New agents receive no access automatically.") {
                if (view.agents.isEmpty()) RText("Connect an agent first.", RType.sans(15f), c.ink2, Modifier.padding(16.dp))
                view.agents.forEach { agent ->
                    CheckRow(agent.connectionId in draft.agentIds, { model.agent(agent.connectionId, it) }, Modifier.fillMaxWidth().padding(16.dp), enabled = !state.busy && !state.changedElsewhere, tag = "permissionAgent:${agent.connectionId}") {
                        Column {
                            RText(untrusted(agent.label), RType.sans(16f), c.ink)
                            RText(if (agent.hasAccountAccess) "Already has account access" else "Will be allowed to discover this account", RType.sans(13f), c.ink2)
                        }
                    }
                }
            }
            if (!finishing || draft.preset == AccountPermissionPreset.CUSTOM) view.capabilities.forEach { capability ->
                val rule = draft.rules.firstOrNull { it.capability == capability.capability }
                if (model.temporary && PermissionMode.AUTO !in capability.supportedModes) return@forEach
                if (!model.temporary && rule == null) return@forEach
                Group(header = untrusted(capability.title)) {
                    Column(Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(10.dp)) {
                        RText(untrusted(capability.description), RType.sans(14f), c.ink2)
                        capability.configuredMode?.let { RText("Current: ${permissionModeTitle(it)}", RType.sans(14f), c.ink) }
                        RText(untrusted(capability.effectiveReason), RType.sans(14f), c.ink)
                        PermissionRestrictions(capability.restrictions)
                        if (model.temporary) CheckRow(rule != null, { model.include(capability.capability, it) }, enabled = !state.busy && view.usageVerified, tag = "delegationCapability:${capability.capability}") {
                            RText("Allow automatically", RType.sans(15f), c.ink)
                        }
                        if (!model.temporary) PermissionMode.entries.forEach { mode ->
                            CapsuleButton(permissionModeTitle(mode), Modifier.fillMaxWidth().testTag("permissionMode:${capability.capability}:$mode"),
                                enabled = !state.busy && !state.changedElsewhere && mode in capability.supportedModes && (mode != PermissionMode.AUTO || view.usageVerified),
                                style = if (rule?.mode == mode) ButtonStyle.Inverse else ButtonStyle.Soft,
                             maxLines = Int.MAX_VALUE) { model.mode(capability.capability, mode) }
                        }
                        if (rule?.mode == PermissionMode.AUTO) {
                            RText("Automatic permission scope", RType.sans(14f), c.ink)
                            RText(permissionScopeTitle(rule.scope.kind) + if (rule.scope.kind == PermissionScopeKind.ALL_IN_ACCOUNT) "" else " · ${rule.scope.ids.size} selected", RType.sans(14f), c.ink2)
                            CapsuleButton("Choose scope", Modifier.fillMaxWidth().testTag("permissionScope:${capability.capability}"), enabled = !state.busy, style = ButtonStyle.Soft, maxLines = Int.MAX_VALUE) { scopeFor = capability.capability }
                        } else {
                            RText("For these operations in this account, for the selected agents.", RType.sans(13f), c.ink2)
                        }
                    }
                }
            }
            if (finishing && draft.preset != AccountPermissionPreset.CUSTOM) Group(header = "Your choices") {
                Column(Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                    draft.rules.forEach { rule ->
                        val capability = view.capabilities.firstOrNull { it.capability == rule.capability }
                        if (capability != null) {
                            RText("${untrusted(capability.title)}: ${permissionModeTitle(rule.mode)}", RType.sans(15f), c.ink)
                            if (rule.mode == PermissionMode.AUTO) RText(untrusted(capability.description), RType.sans(13f), c.ink2)
                        }
                    }
                }
            }
            if (draft.rules.any { it.mode == PermissionMode.AUTO }) {
                Group(header = "Automatic permission limits") {
                    Column(Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(10.dp)) {
                        RText("Expires: ${permissionExpiry(draft.expiresAt)}", RType.sans(14f), c.ink, Modifier.testTag("permissionExpiry"))
                        RText("Uses available: ${if (view.usageVerified) draft.maxUses?.toString() ?: "Not set" else "Unavailable"}", RType.sans(14f), c.ink)
                        val needsLimits = draft.expiresAt == null || draft.maxUses == null
                        if (needsLimits) RText("Choose an expiry and enter a use limit before reviewing automatic permission.", RType.sans(14f), c.ink2)
                        if (!editLimits && !needsLimits) CapsuleButton("Change limits", Modifier.fillMaxWidth(), style = ButtonStyle.Soft, maxLines = Int.MAX_VALUE) { editLimits = true }
                        if (editLimits || needsLimits) {
                        listOf(3600L to "1 hour", 28800L to "8 hours", 86400L to "1 day", 604800L to "7 days").filter { it.first.toULong() <= if (model.temporary) view.delegationMaximumDurationSecs else view.maximumDurationSecs }.forEach { (seconds, label) ->
                            CapsuleButton("Expire in $label", Modifier.fillMaxWidth(), enabled = !state.busy, style = ButtonStyle.Soft, maxLines = Int.MAX_VALUE) { model.expiresAt(System.currentTimeMillis() / 1000 + seconds) }
                        }
                        RText("Remaining uses to authorize", RType.sans(14f), c.ink)
                        RTextField(if (view.usageVerified) draft.maxUses?.toString().orEmpty() else "", { model.remainingUses(it.toULongOrNull()) }, "Number of uses", tag = "permissionUses", enabled = !state.busy && view.usageVerified, keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Number))
                        }
                        RText(if (view.usageVerified) "Already used: ${view.used}. Limits are enforced by the core; unknown outcomes are not silently refunded." else "Usage is unavailable until history is verified.", RType.sans(13f), c.ink2)
                    }
                }
            }
            Column(Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(12.dp)) {
                CapsuleButton("Review changes", Modifier.fillMaxWidth().testTag("permissionsPreview"), enabled = !state.busy && !state.changedElsewhere && (view.usageVerified || draft.rules.none { it.mode == PermissionMode.AUTO }), busy = state.busy, onClick = model::preview, maxLines = Int.MAX_VALUE)
                if (!finishing && !model.temporary) CapsuleButton("Create a temporary permission", Modifier.fillMaxWidth().testTag("accountDelegate"), enabled = !state.busy && !state.changedElsewhere && view.usageVerified && view.capabilities.any { PermissionMode.AUTO in it.supportedModes }, style = ButtonStyle.Soft, onClick = onDelegate, maxLines = Int.MAX_VALUE)
            }
            }
        }
        if (finishing && !state.saved) Column(Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(12.dp)) {
            CapsuleButton(if (view != null && !model.canEdit) "Done" else "Later — keep current permissions", Modifier.fillMaxWidth().testTag("connectionDone"), enabled = !state.busy, style = ButtonStyle.Soft, onClick = onDone, maxLines = Int.MAX_VALUE)
            onAnother?.let { CapsuleButton("Add another account", Modifier.fillMaxWidth().testTag("connectionAddAnother"), enabled = !state.busy, style = ButtonStyle.Soft, onClick = it, maxLines = Int.MAX_VALUE) }
        }
    }
}

@Composable
fun PermissionRestrictions(restrictions: List<PermissionRestriction>) {
    val c = LocalColors.current
    restrictions.distinct().forEach { restriction ->
        RText("${permissionControllerTitle(restriction.controller)}: ${untrusted(restriction.reason)}", RType.sans(13f), c.ink2)
    }
}

fun permissionPreviewText(preview: PermissionPreview): String = buildString {
    append(untrusted(preview.summary))
    preview.warnings.forEach { append("\n\n"); append(untrusted(it)) }
    append("\n\nRetained history: automatic "); append(preview.eligibleHistoryCount?.toString() ?: "unavailable")
    append(", review "); append(preview.reviewHistoryCount?.toString() ?: "unavailable")
    append(", blocked "); append(preview.blockedHistoryCount?.toString() ?: "unavailable")
    if (!preview.historyComplete) append("\nHistory coverage is partial. These counts are not a prediction of future requests.")
}

@Composable
private fun PermissionScopePicker(model: AccountPermissionsViewModel, capability: PermissionCapability, onBack: () -> Unit) {
    val state by model.ui.collectAsStateWithLifecycle()
    val c = LocalColors.current
    var query by remember(capability) { mutableStateOf("") }
    BackHandler(onBack = onBack)
    LaunchedEffect(capability, query) { model.scopeOptions(capability, query) }
    val selected = state.draft?.rules?.firstOrNull { it.capability == capability }?.scope
    Screen(title = "Permission scope", subtitle = untrusted(model.account), onBack = onBack) {
        Column(Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(12.dp)) {
            if (state.optionsLoading) RText("Loading scope choices…", RType.sans(14f), c.ink2)
            state.options?.let { options ->
                options.unsupportedReason?.let { Banner(untrusted(it), kind = BannerKind.Warning) }
                options.kinds.forEach { kind ->
                    CapsuleButton(permissionScopeTitle(kind), Modifier.fillMaxWidth().testTag("scopeKind:$kind"), style = if (selected?.kind == kind) ButtonStyle.Inverse else ButtonStyle.Soft, maxLines = Int.MAX_VALUE) {
                        if (selected?.kind != kind) model.scope(capability, PermissionScopeSelection(kind, emptyList()))
                    }
                }
                if (selected?.kind != PermissionScopeKind.ALL_IN_ACCOUNT) {
                    RTextField(query, { query = it }, if (options.allowsAddressEntry) "Search or enter an email address" else "Search messages", tag = "permissionScopeQuery")
                    options.options.filter { it.service == model.service && it.account == model.account && it.kind == selected?.kind }.forEach { option ->
                        CheckRow(option.id in selected!!.ids, { checked ->
                            model.scope(capability, selected.copy(ids = if (checked) (selected.ids + option.id).distinct() else selected.ids - option.id))
                        }, Modifier.fillMaxWidth().padding(vertical = 10.dp), enabled = !state.optionsLoading, tag = "scopeOption:${option.id}") {
                            Column {
                                RText(untrusted(option.label), RType.sans(15f), c.ink)
                                if (option.detail.isNotBlank()) RText(untrusted(option.detail), RType.sans(13f), c.ink2)
                            }
                        }
                        Hairline()
                    }
                }
            }
            state.error?.let { Banner(untrusted(it), kind = BannerKind.Error) }
            CapsuleButton("Done", Modifier.fillMaxWidth().testTag("permissionScopeDone"), onClick = onBack, maxLines = Int.MAX_VALUE)
        }
    }
}
