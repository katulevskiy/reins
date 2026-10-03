package dev.reins.android.push

/** A validated FCM data message (contracts §A): `t` is `req`, `pair`, `blob`, `join` or `replaced`, `id` a server-issued id. */
data class PushPayload(val kind: String, val id: String) {
    companion object {
        private val ID = Regex("[A-Za-z0-9_-]{1,128}")
        private val KINDS = setOf("req", "pair", "blob", "join", "replaced")

        /** Null for anything that is not exactly what the server sends; push is only a hint, so drop the rest. */
        fun parse(data: Map<String, String>): PushPayload? {
            val kind = data["t"] ?: return null
            val id = data["id"] ?: return null
            if (kind !in KINDS || !ID.matches(id)) return null
            return PushPayload(kind, id)
        }
    }
}
