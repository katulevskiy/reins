package dev.rewarden.android.ui.signin

/** How hard a master password looks to guess: a hint only; the server's one rule is the length. */
enum class Strength { Weak, Fair, Strong }

/** What the onboarding forms check before they call the core. Pure, so it is unit-tested. */
object AccountRules {
    /** The server refuses shorter master passwords. */
    const val MIN_PASSWORD = 12

    /**
     * A simple heuristic: shorter than [MIN_PASSWORD] or very repetitive is weak; long, or 16+ characters mixing three
     * kinds (lower case, upper case, digits, other), is strong; anything else is fair.
     */
    fun strength(password: String): Strength {
        val length = password.codePointCount(0, password.length)
        if (length < MIN_PASSWORD) return Strength.Weak
        if (password.toSet().size < 5) return Strength.Weak
        val kinds = listOf<(Char) -> Boolean>(Char::isLowerCase, Char::isUpperCase, Char::isDigit, { c: Char -> !c.isLetterOrDigit() })
            .count { kind -> password.any(kind) }
        return when {
            length >= 20 -> Strength.Strong
            length >= 16 && kinds >= 3 -> Strength.Strong
            else -> Strength.Fair
        }
    }

    /** One line under the strength label. */
    fun strengthHint(password: String): String = when {
        password.codePointCount(0, password.length) < MIN_PASSWORD -> "Use at least $MIN_PASSWORD characters."
        strength(password) == Strength.Weak -> "Too repetitive. Mix in other characters."
        strength(password) == Strength.Fair -> "Longer, or mixing letters, digits and symbols, is stronger."
        else -> "Hard to guess. Remember it, or write it down."
    }

    /** Good enough to try: the server checks the rest. */
    fun isEmail(email: String): Boolean {
        val e = email.trim()
        val at = e.indexOf('@')
        return at > 0 && at == e.lastIndexOf('@') && e.indexOf('.', at) > at + 1 && !e.endsWith('.') && e.none { it.isWhitespace() }
    }

    /**
     * Why "Create account" cannot be pressed yet, or null when it can. The first unmet rule, in the order of the form.
     */
    fun createProblem(email: String, password: String, confirm: String, termsAccepted: Boolean): String? = when {
        !isEmail(email) -> "Enter your email address."
        password.codePointCount(0, password.length) < MIN_PASSWORD -> "The master password needs at least $MIN_PASSWORD characters."
        confirm != password -> "The passwords don't match."
        !termsAccepted -> "Accept the Terms to continue."
        else -> null
    }

    /** What the user typed as a server, with `https://` added when there is no scheme; null when it is no address. */
    fun serverUrl(input: String): String? {
        val s = input.trim()
        val withScheme = if ("://" in s) s else "https://$s"
        val scheme = withScheme.substringBefore("://").lowercase(java.util.Locale.ROOT)
        val rest = withScheme.substringAfter("://").trimEnd('/')
        if (scheme != "https" && scheme != "http") return null
        if (rest.isEmpty() || rest.startsWith('/') || rest.any { it.isWhitespace() }) return null
        return "$scheme://$rest"
    }

    /** "app.reins2fa.com" for "https://app.reins2fa.com/". */
    fun displayHost(server: String): String = server.trim().substringAfter("://").trimEnd('/')

    /** Where AI apps reach Reins on [server]: its `/mcp` endpoint. */
    fun mcpUrl(server: String): String = server.trim().trimEnd('/') + "/mcp"
}
