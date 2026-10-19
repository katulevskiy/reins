package dev.reins.android.ui.permissions

import dev.reins.core.AccountPermissionPreset
import dev.reins.core.PermissionController
import dev.reins.core.PermissionMode
import dev.reins.core.PermissionScopeKind
import java.time.Instant
import java.time.ZoneId
import java.time.format.DateTimeFormatter
import java.time.format.FormatStyle

fun permissionModeTitle(mode: PermissionMode): String = when (mode) {
    PermissionMode.ASK -> "Review by default"
    PermissionMode.AUTO -> "Allow automatically"
    PermissionMode.BLOCK -> "Block"
}

fun permissionPresetTitle(preset: AccountPermissionPreset): String = when (preset) {
    AccountPermissionPreset.REVIEW_EVERY_REQUEST -> "Review by default"
    AccountPermissionPreset.READ_ASSIST -> "Allow searching and reading"
    AccountPermissionPreset.CUSTOM -> "Customize"
}

fun permissionControllerTitle(controller: PermissionController): String = when (controller) {
    PermissionController.OWNER -> "Your policy"
    PermissionController.ORGANIZATION -> "Organization policy"
    PermissionController.UNSUPPORTED_ADAPTER -> "Integration limitation"
    PermissionController.CONFLICT -> "Policy conflict"
}

fun permissionScopeTitle(kind: PermissionScopeKind): String = when (kind) {
    PermissionScopeKind.ALL_IN_ACCOUNT -> "All in this account"
    PermissionScopeKind.SELECTED_RECIPIENTS -> "Selected recipients"
    PermissionScopeKind.SELECTED_RESOURCES -> "Selected messages"
}

fun permissionExpiry(at: Long?): String = at?.let {
    runCatching { DateTimeFormatter.ofLocalizedDateTime(FormatStyle.MEDIUM).format(Instant.ofEpochSecond(it).atZone(ZoneId.systemDefault())) }
        .getOrDefault("Expiry unavailable")
} ?: "Choose an expiry"
