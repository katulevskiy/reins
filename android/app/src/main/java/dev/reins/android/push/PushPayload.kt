package dev.reins.android.push

/**
 * A validated FCM data message (contracts §A): `t` is `req`, `pair`, `blob`, `join`, `replaced`, `orgreview` or `circle`, `id` a
 * server-issued id (a Circle's is `review:`, `alert:` or `news:` and an id; the core then syncs Circles), and `p` (optional) the tag of the account it is for (`reins_proto::pairing::push_profile_tag`), so a phone with a
 * work profile knows which profile a push belongs to.
 */
data class PushPayload(val kind: String, val id: String, val profileTag: String? = null) {
    companion object {
        private val ID = Regex("[A-Za-z0-9_-]{1,128}")
        private val TAG = Regex("[0-9a-f]{16}")
        /** An org review waits for this member (O2): the app shows it; the core fetches nothing. */
        const val ORG_REVIEW = "orgreview"
        /** Something about Circles is new (a review, a guardian notice, a change): the core syncs them. */
        const val CIRCLE = "circle"
        private val CIRCLE_ID = Regex("(review|alert|news):[A-Za-z0-9_-]{1,120}")
        private val KINDS = setOf("req", "pair", "blob", "join", "replaced", ORG_REVIEW, CIRCLE)

        /** Null for anything that is not exactly what the server sends; push is only a hint, so drop the rest. */
        fun parse(data: Map<String, String>): PushPayload? {
            val kind = data["t"] ?: return null
            val id = data["id"] ?: return null
            if (kind !in KINDS || !(if (kind == CIRCLE) CIRCLE_ID else ID).matches(id)) return null
            val tag = data["p"]?.takeIf { it.isNotEmpty() }
            if (tag != null && !TAG.matches(tag)) return null
            return PushPayload(kind, id, tag)
        }
    }
}
