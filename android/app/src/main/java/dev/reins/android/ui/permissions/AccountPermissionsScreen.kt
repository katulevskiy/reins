package dev.reins.android.ui.permissions

import androidx.compose.foundation.layout.heightIn
import androidx.compose.ui.Alignment
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.selected
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.foundation.selection.selectable
import androidx.compose.ui.draw.clip
import androidx.compose.foundation.background
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
import androidx.compose.foundation.layout.Row
import dev.reins.android.design.Glyph
import dev.reins.android.design.GlyphIcon
import dev.reins.android.design.RowTile
import dev.reins.android.design.SignalRadio
import dev.reins.android.design.SwitchRow
import dev.reins.android.design.pressable
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
                RText(untrusted(preview.summary), RType.sans(15f, lineHeight = 23f), c.ink, Modifier.testTag("permissionPreviewSummary"))
                if (state.account != null && state.draft != null) PermissionDraftReview(state.account!!, state.draft!!, model.temporary)
                preview.warnings.forEach { RText(untrusted(it), RType.sans(14f), c.ink2) }
                RText(permissionHistoryText(preview), RType.sans(14f), c.ink2, Modifier.testTag("permissionPreviewHistory"))
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
                    RText("${untrusted(capability.title)}: ${untrusted(capability.scopeSummary)}", RType.sans(14f), c.ink)
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
        if (view?.service == "gmail" && view.capabilities.none { PermissionMode.AUTO in it.supportedModes }) PrimaryPhoneSetupSection(model, authenticator)
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
            // Next (Permissions 02): where the rules start, as radio rows.
            if (!model.temporary) Group(header = "Start from") {
                AccountPermissionPreset.entries.forEachIndexed { i, preset ->
                    if (i > 0) Hairline()
                    val on = (!finishing || state.hasChosenPreset) && draft.preset == preset
                    RadioRow(
                        permissionPresetTitle(preset),
                        presetGlyph(preset),
                        on,
                        enabled = !state.busy && !state.changedElsewhere && preset in view.availablePresets && (preset != AccountPermissionPreset.READ_ASSIST || view.usageVerified),
                        tag = "permissionPreset:$preset",
                    ) { model.preset(preset) }
                }
            }
            if (!finishing || state.hasChosenPreset) {
            RText("Review by default asks for confirmation unless a matching explicit temporary permission allows the action. Mandatory review and blocks still apply.", RType.sans(14f), c.ink2, Modifier.padding(horizontal = 16.dp))
            Group(header = "Agents", footer = "Automatic permissions require selected agents. With no selection, review and block rules apply to all agents, including future connections. Selecting an agent without access explicitly admits it to this account.") {
                if (draft.agentIds.isEmpty()) RText("All agents (including future connections) for review and block rules. Automatic permission requires selected agents.", RType.sans(14f), c.ink2, Modifier.padding(16.dp).testTag("permissionAllAgents"))
                if (view.agents.isEmpty()) RText("No agents are connected. You can still set review or block defaults.", RType.sans(15f), c.ink2, Modifier.padding(16.dp))
                view.agents.forEachIndexed { i, agent ->
                    if (i > 0) Hairline()
                    // Next: each agent with its switch; the line under says what turning it on admits.
                    SwitchRow(
                        untrusted(agent.label),
                        agent.connectionId in draft.agentIds,
                        { model.agent(agent.connectionId, it) },
                        subtitle = if (agent.hasAccountAccess) "Already has account access" else "Will be allowed to discover this account",
                        glyph = Glyph.Robot,
                        enabled = !state.busy && !state.changedElsewhere,
                        tag = "permissionAgent:${agent.connectionId}",
                    )
                }
            }
            if (!finishing || draft.preset == AccountPermissionPreset.CUSTOM) view.capabilities.forEach { capability ->
                val rule = draft.rules.firstOrNull { it.capability == capability.capability }
                if (model.temporary && PermissionMode.AUTO !in capability.supportedModes) return@forEach
                if (!model.temporary && rule == null) return@forEach
                Group(header = untrusted(capability.title)) {
                    Column(Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(10.dp)) {
                        RText(untrusted(capability.description), RType.sans(14f), c.ink2)
                        RText("Current settings: ${untrusted(capability.scopeSummary)}", RType.sans(14f), c.ink)
                        RText(untrusted(capability.effectiveReason), RType.sans(14f), c.ink)
                        PermissionRestrictions(capability.restrictions)
                        if (model.temporary) CheckRow(rule != null, { model.include(capability.capability, it) }, enabled = !state.busy && view.usageVerified, tag = "delegationCapability:${capability.capability}") {
                            RText("Allow automatically", RType.sans(15f), c.ink)
                        }
                        // Next (Permissions 03): Review, Allow or Block in one well; the chosen one raised.
                        if (!model.temporary) Row(
                            Modifier.fillMaxWidth().clip(androidx.compose.foundation.shape.CircleShape).background(c.paperSunken).padding(4.dp),
                            horizontalArrangement = Arrangement.spacedBy(2.dp),
                        ) {
                            PermissionMode.entries.forEach { mode ->
                                val on = rule?.mode == mode
                                val enabled = !state.busy && !state.changedElsewhere && mode in capability.supportedModes && (mode != PermissionMode.AUTO || view.usageVerified)
                                Row(
                                    Modifier
                                        .weight(1f)
                                        .heightIn(min = 40.dp)
                                        .clip(androidx.compose.foundation.shape.CircleShape)
                                        .background(if (on) c.paperRaised else androidx.compose.ui.graphics.Color.Transparent)
                                        .pressable(enabled = enabled, shape = androidx.compose.foundation.shape.CircleShape, label = permissionModeTitle(mode)) { model.mode(capability.capability, mode) }
                                        .semantics { selected = on }
                                        .testTag("permissionMode:${capability.capability}:$mode")
                                        .graphicsLayer { alpha = if (enabled) 1f else 0.4f }
                                        .padding(horizontal = 6.dp),
                                    horizontalArrangement = Arrangement.spacedBy(4.dp, Alignment.CenterHorizontally),
                                    verticalAlignment = Alignment.CenterVertically,
                                ) {
                                    GlyphIcon(modeGlyph(mode), if (on) c.ink else c.ink2, size = 15.dp)
                                    RText(shortModeTitle(mode), RType.sans(14f, FontWeight.SemiBold), if (on) c.ink else c.ink2, maxLines = 1)
                                }
                            }
                        }
                        if (rule?.mode == PermissionMode.AUTO) {
                            RText("Automatic permission scope", RType.sans(14f), c.ink)
                            RText(permissionScopeTitle(rule.scope.kind) + if (rule.scope.kind == PermissionScopeKind.ALL_IN_ACCOUNT) "" else " · ${rule.scope.ids.size} selected", RType.sans(14f), c.ink2)
                            CapsuleButton("Choose scope", Modifier.fillMaxWidth().testTag("permissionScope:${capability.capability}"), enabled = !state.busy, style = ButtonStyle.Soft, maxLines = Int.MAX_VALUE) { scopeFor = capability.capability }
                        } else {
                            RText(if (draft.agentIds.isEmpty()) "For these operations in this account, for all agents including future connections." else "For these operations in this account, for the selected agents.", RType.sans(13f), c.ink2)
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
                        RText("${if (model.temporary) "Total uses available" else "Further uses per automatic operation"}: ${if (view.usageVerified) draft.maxUses?.toString() ?: "Not set" else "Unavailable"}", RType.sans(14f), c.ink)
                        val needsLimits = draft.expiresAt == null || draft.maxUses == null
                        if (needsLimits) RText("Choose an expiry and enter a use limit before reviewing automatic permission.", RType.sans(14f), c.ink2)
                        if (!editLimits && !needsLimits) CapsuleButton("Change limits", Modifier.fillMaxWidth(), style = ButtonStyle.Soft, maxLines = Int.MAX_VALUE) { editLimits = true }
                        if (editLimits || needsLimits) {
                        listOf(3600L to "1 hour", 28800L to "8 hours", 86400L to "1 day", 604800L to "7 days").filter { it.first.toULong() <= if (model.temporary) view.delegationMaximumDurationSecs else view.maximumDurationSecs }.forEach { (seconds, label) ->
                            CapsuleButton("Expire in $label", Modifier.fillMaxWidth(), enabled = !state.busy, style = ButtonStyle.Soft, maxLines = Int.MAX_VALUE) { model.expiresAt(System.currentTimeMillis() / 1000 + seconds) }
                        }
                        RText(if (model.temporary) "Total uses to authorize" else "Further uses to authorize for each automatic operation", RType.sans(14f), c.ink)
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

/** A radio row (Next `.rad`): its glyph's tile, the words, and the radio at the end. */
@Composable
private fun RadioRow(title: String, glyph: Glyph, on: Boolean, enabled: Boolean, tag: String, onClick: () -> Unit) {
    val c = LocalColors.current
    Row(
        Modifier
            .fillMaxWidth()
            .heightIn(min = 56.dp)
            .selectable(on, enabled = enabled, role = androidx.compose.ui.semantics.Role.RadioButton, onClick = onClick)
            .testTag(tag)
            .graphicsLayer { alpha = if (enabled) 1f else 0.4f }
            .padding(vertical = 9.dp),
        horizontalArrangement = Arrangement.spacedBy(12.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        RowTile(glyph)
        RText(title, RType.row, c.ink, Modifier.weight(1f), maxLines = 2)
        SignalRadio(on)
    }
}

private fun presetGlyph(preset: AccountPermissionPreset): Glyph = when (preset.name) {
    "REVIEW_EVERY_REQUEST" -> Glyph.Person
    "READ_ASSIST" -> Glyph.Eye
    else -> Glyph.Settings
}

private fun modeGlyph(mode: PermissionMode): Glyph = when (mode) {
    PermissionMode.AUTO -> Glyph.Check
    PermissionMode.BLOCK -> Glyph.Close
    else -> Glyph.Person
}

/** "Review", "Allow", "Block": a mode in one word for the well. */
private fun shortModeTitle(mode: PermissionMode): String = when (mode) {
    PermissionMode.AUTO -> "Allow"
    PermissionMode.BLOCK -> "Block"
    else -> "Review"
}

@Composable
fun PermissionRestrictions(restrictions: List<PermissionRestriction>) {
    val c = LocalColors.current
    restrictions.distinct().forEach { restriction ->
        RText("${permissionControllerTitle(restriction.controller)}: ${untrusted(restriction.reason)}", RType.sans(13f), c.ink2)
    }
}

fun permissionHistoryText(preview: PermissionPreview): String {
    if (preview.eligibleHistoryCount == null && preview.reviewHistoryCount == null && preview.blockedHistoryCount == null) return "A history comparison is unavailable."
    return "Past requests: ${preview.eligibleHistoryCount?.toString() ?: "unavailable"} automatic, ${preview.reviewHistoryCount?.toString() ?: "unavailable"} review, ${preview.blockedHistoryCount?.toString() ?: "unavailable"} blocked." +
        if (preview.historyComplete) "" else " Only part of your history is available. Past results do not predict future requests."
}

/** Displays only the exact typed draft already validated by the core preview. */
@Composable
private fun PermissionDraftReview(view: dev.reins.core.AccountPermissionsView, draft: dev.reins.core.AccountPermissionsDraft, temporary: Boolean) {
    val c = LocalColors.current
    fun label(id: String) = view.agents.firstOrNull { it.connectionId == id }?.label ?: id
    Column(verticalArrangement = Arrangement.spacedBy(10.dp), modifier = Modifier.testTag("permissionReviewDetails")) {
        RText("Account", RType.sans(15f), c.ink2)
        RText("${dev.reins.android.design.serviceName(draft.service)} · ${untrusted(draft.account)}", RType.sans(16f), c.ink, ltr = true)
        RText("Agents", RType.sans(15f), c.ink2)
        if (draft.agentIds.isEmpty()) RText("All agents (including future connections)", RType.sans(16f), c.ink)
        draft.agentIds.forEach { id ->
            RText(untrusted(label(id)), RType.sans(16f), c.ink)
            RText(untrusted(id), RType.sans(13f), c.ink2, ltr = true)
        }
        val admitted = draft.agentIds.filter { id -> view.agents.none { it.connectionId == id && it.hasAccountAccess } }
        if (draft.grantAccountAccess && admitted.isNotEmpty()) {
            RText("New account access", RType.sans(15f), c.ink2)
            admitted.forEach { RText("${untrusted(label(it))} can discover this account.", RType.sans(15f), c.ink) }
            RText("Account access expires: ${draft.expiresAt?.let { permissionExpiry(it) } ?: "No expiry"}", RType.sans(14f), c.ink2)
        } else RText("No additional agents receive account access.", RType.sans(14f), c.ink2)
        draft.rules.forEach { rule ->
            Hairline()
            val title = view.capabilities.firstOrNull { it.capability == rule.capability }?.title ?: rule.capability.name
            RText("${untrusted(title)}: ${permissionModeTitle(rule.mode)}", RType.sans(16f), c.ink)
            if (rule.mode == PermissionMode.AUTO) {
                RText(permissionScopeTitle(rule.scope.kind), RType.sans(14f), c.ink2)
                rule.scope.ids.forEach { RText(untrusted(it), RType.sans(15f), c.ink, ltr = true) }
                if (rule.scope.kind == PermissionScopeKind.SELECTED_RECIPIENTS) RText("Every To and Cc address must match these recipients.", RType.sans(13f), c.ink2)
            } else RText(if (draft.agentIds.isEmpty()) "All requests of this operation in this account, from all agents including future connections." else "All requests of this operation in this account, from the selected agents.", RType.sans(14f), c.ink2)
        }
        if (draft.rules.any { it.mode == PermissionMode.AUTO }) Column(verticalArrangement = Arrangement.spacedBy(8.dp), modifier = Modifier.testTag("permissionReviewLimits")) {
            Hairline()
            RText("Automatic permission expires: ${permissionExpiry(draft.expiresAt)}", RType.sans(15f), c.ink)
            RText(if (temporary) "At most ${draft.maxUses} total requests across the selected operations." else "At most ${draft.maxUses} further uses for each automatic operation.", RType.sans(15f), c.ink)
        }
    }
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
