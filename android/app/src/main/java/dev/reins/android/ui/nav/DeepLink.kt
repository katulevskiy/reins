package dev.reins.android.ui.nav

import dev.reins.core.PendingItem
import dev.reins.core.PendingKind

/** An `OPEN_ITEM` intent. Any app can send one to the exported launcher activity, so nothing in it is trusted. */
data class DeepLink(val kind: PendingKind, val id: String) {
    companion object {
        private val ID = Regex("[A-Za-z0-9_-]{1,128}")

        fun isId(value: String) = ID.matches(value)

        fun parse(action: String?, kind: String?, id: String?, expectedAction: String): DeepLink? {
            if (action != expectedAction || id == null || !ID.matches(id)) return null
            val parsed = when (kind) {
                "request" -> PendingKind.REQUEST
                "pairing" -> PendingKind.PAIRING
                "blob" -> PendingKind.BLOB
                "join" -> PendingKind.JOIN
                else -> return null
            }
            return DeepLink(parsed, id)
        }
    }

    /** Only items the core really has parked can be opened. */
    fun resolve(pending: List<PendingItem>): SheetTarget? {
        val item = pending.firstOrNull { it.kind == kind && it.id == id } ?: return null
        return when (item.kind) {
            PendingKind.REQUEST -> SheetTarget.Approval(item.id)
            PendingKind.PAIRING -> SheetTarget.Pairing(item.id)
            PendingKind.BLOB -> SheetTarget.Upload(item.id)
            PendingKind.JOIN -> SheetTarget.Join(item.id)
        }
    }
}
