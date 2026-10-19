package dev.reins.android.ui.approval

import dev.reins.android.autopilot.AutopilotText
import dev.reins.android.design.Glyph
import dev.reins.android.ui.common.untrusted
import dev.reins.core.ApprovalView
import dev.reins.core.RiskView
import dev.reins.core.SuggestionView

/** How a warning line reads: red for what is held and asked every time, amber for what to check, plain for the rest. */
enum class WarnTone { Red, Amber, Plain }

/**
 * One warning line on the field: its own words, a glyph, and (for some) a longer reason behind its (?). [maxLines] is
 * unlimited except for the risk, which wraps to two lines and then gives its reason to the first fact row.
 */
data class FieldWarning(
    val tag: String,
    val text: String,
    val tone: WarnTone,
    val help: String? = null,
    val maxLines: Int = Int.MAX_VALUE,
) {
    val glyph: Glyph get() = when (tone) {
        WarnTone.Red -> Glyph.Lock
        WarnTone.Amber -> Glyph.Warning
        WarnTone.Plain -> Glyph.ShieldAlert
    }
}

/**
 * Every warning a request carries, as the field shows them, each on a line of its own, in this order:
 * - each danger tag ([dangerTags]): the guard rule, a command this phone couldn't read, a push that rewrites history
 *   or that couldn't be checked, hidden characters, a destructive tool, what can't be undone;
 * - that the computer reported the rule (this phone doesn't know it);
 * - a push to the repository's default branch (display only), an SSH server with no name;
 * - why it is asked every time, when no danger tag already says so;
 * - the first time this AI asks for this target;
 * - the risk Overnight's model saw (two lines; its word only once the reason went to the facts), and that it held it.
 * Nothing here is behind a fold or a (?) only.
 */
fun fieldWarnings(view: ApprovalView, suggestion: SuggestionView?, risk: RiskView?, riskInFacts: Boolean = false): List<FieldWarning> = buildList {
    val dangers = dangerTags(view)
    dangers.forEach { d -> add(FieldWarning(d.tag, dangerLine(d), if (d.destructive) WarnTone.Red else WarnTone.Amber, d.help)) }
    val rule = view.ask?.rule
    if (rule != null && rule.reported) {
        add(FieldWarning("ruleReported", "Your computer says so; this phone doesn't know the rule", WarnTone.Amber))
    }
    defaultBranchPush(view)?.let { add(FieldWarning("defaultBranch", "To the default branch ${untrusted(it)}", WarnTone.Amber)) }
    view.ssh?.takeIf { it.host.isNullOrBlank() }?.let { ssh ->
        add(FieldWarning("unknownHost", if (ssh.hostKey.isNullOrBlank()) "Unknown server" else "Server known only by its key", WarnTone.Amber))
    }
    if (dangers.isEmpty()) {
        if (mailPermissionSettings(view)) add(FieldWarning("alwaysAsks", "This approval is once only. Manage ongoing access in Trust.", WarnTone.Plain))
        else alwaysAskReason(view)?.let { add(FieldWarning("alwaysAsks", "Always asks · $it", WarnTone.Plain)) }
    }
    if (suggestion?.novel == true) add(FieldWarning("novel", "First time for this AI", WarnTone.Amber))
    if (risk != null) {
        val line = if (riskInFacts) "Risk: ${AutopilotText.riskWord(risk.level)}" else AutopilotText.riskLine(risk)
        add(FieldWarning("risk", line, WarnTone.Amber, maxLines = 2))
        AutopilotText.heldLine(risk)?.let { add(FieldWarning("riskHeld", it, WarnTone.Amber)) }
    }
}

/** The branch a push sends to when it is the repository's default one, as the push names it; null otherwise. */
fun defaultBranchPush(view: ApprovalView): String? {
    val push = view.git ?: return null
    val default = push.defaultBranch?.takeIf { it.isNotBlank() } ?: return null
    return push.refs.firstOrNull { it.kind == "branch" && it.shortName == default }?.shortName
}

/** Why nothing here may be remembered, in a few words; null when it may be. */
fun alwaysAskReason(view: ApprovalView): String? {
    if (!rememberLocked(view)) return null
    val env = view.envRelease
    return when {
        view.purchase != null -> "money can't come back"
        env != null && env.print -> "it prints a value"
        env != null && env.alwaysAsk -> "your computer asks for it every time"
        env != null -> "this command can't be remembered"
        view.ssh != null -> "it signs in to a server"
        view.service == "vault" -> "it hands over a password"
        else -> "it's never remembered"
    }
}

/** Any red line holds the action and locks the chooser to "Asked every time". */
fun heldByDanger(view: ApprovalView): Boolean = destructive(view) || dangerTags(view).any { it.destructive }
