package dev.reins.android

import dev.reins.core.*

/** Synthetic DTO fixtures only. Production rules and canonicalization are supplied by reins-core. */
object PermissionFixtures {
    fun view(service: String, account: String, revision: ULong = 1u): AccountPermissionsView {
        val gmail = service == "gmail"
        val modes = if (gmail) listOf(PermissionMode.ASK, PermissionMode.AUTO, PermissionMode.BLOCK) else emptyList()
        val capabilities = if (gmail) listOf(PermissionCapability.MAIL_SEARCH, PermissionCapability.MAIL_READ, PermissionCapability.MAIL_SEND) else listOf(PermissionCapability.INTEGRATION_READ, PermissionCapability.INTEGRATION_WRITE)
        return AccountPermissionsView(service, account, "$service/$account", revision, gmail, false,
            capabilities.map { capability ->
                val scope = if (capability == PermissionCapability.MAIL_SEND) PermissionScopeKind.SELECTED_RECIPIENTS else PermissionScopeKind.ALL_IN_ACCOUNT
                PermissionCapabilityView(capability, when (capability) { PermissionCapability.MAIL_SEARCH -> "Search mail"; PermissionCapability.MAIL_READ -> "Read mail"; PermissionCapability.MAIL_SEND -> "Send mail"; else -> "Integration operations" },
                    "Only this account and the selected agents are included.", modes, if (capability == PermissionCapability.MAIL_SEND) listOf(scope) else listOf(scope, PermissionScopeKind.SELECTED_RESOURCES), if (gmail) PermissionMode.ASK else null,
                    PermissionScopeSelection(scope, emptyList()), if (gmail) "Review required for selected agents" else "Existing permissions apply", if (gmail) "New account: review required." else "Automatic permissions are not supported for this integration.", if (gmail) emptyList() else listOf(PermissionRestriction(PermissionController.UNSUPPORTED_ADAPTER, null, "Automatic permissions are not supported for this integration.")))
            }, listOf(PermissionAgentView("c1", "Claude Code", true, true), PermissionAgentView("c2", "Personal agent", false, false)),
            if (gmail) AccountPermissionPreset.entries else emptyList(), null, 604800u, 86400u, 3u, true, null, if (gmail) "Review required. Existing grants and mandatory restrictions still apply." else "Existing grants and explicit restrictions remain in effect; account defaults are read-only.", emptyList())
    }
    fun draft(view: AccountPermissionsView, preset: AccountPermissionPreset) = AccountPermissionsDraft(view.service, view.account, view.revision, preset,
        view.agents.filter { it.selected }.map { it.connectionId }, false,
        if (preset == AccountPermissionPreset.READ_ASSIST) (dev.reins.android.design.Timers.frozenNowMillis ?: System.currentTimeMillis())/1000 + 3600 else view.expiresAt,
        if (preset == AccountPermissionPreset.READ_ASSIST) 20u else view.remainingUses,
        view.capabilities.map { CapabilityPermissionInput(it.capability, if (preset == AccountPermissionPreset.READ_ASSIST && it.capability != PermissionCapability.MAIL_SEND) PermissionMode.AUTO else it.configuredMode ?: if (PermissionMode.ASK in it.supportedModes) PermissionMode.ASK else PermissionMode.BLOCK, it.scope) })
    fun preview(revision: ULong) = PermissionPreview(revision, "Selected agents may search and read this account within the displayed scope and limits. Sending remains subject to review unless explicitly included.", listOf("Sensitive-data protections and other review requirements still apply."), null, null, null, false)
}
