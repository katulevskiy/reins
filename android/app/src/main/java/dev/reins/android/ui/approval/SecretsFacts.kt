package dev.reins.android.ui.approval

import androidx.compose.animation.AnimatedVisibility
import androidx.compose.animation.core.animateFloatAsState
import androidx.compose.animation.expandVertically
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.animation.shrinkVertically
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ExperimentalLayoutApi
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.defaultMinSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.rounded.Add
import androidx.compose.material.icons.rounded.ChevronRight
import androidx.compose.material.icons.rounded.ContentCopy
import androidx.compose.material.icons.rounded.DriveFileRenameOutline
import androidx.compose.material.icons.rounded.Key
import androidx.compose.material.icons.rounded.Person
import androidx.compose.material.icons.rounded.Remove
import androidx.compose.material.icons.rounded.SmartToy
import androidx.compose.material.icons.rounded.Sync
import androidx.compose.material3.Icon
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.draw.rotate
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import dev.reins.android.design.LocalColors
import dev.reins.android.design.RText
import dev.reins.android.design.RType
import dev.reins.android.design.ServiceAvatar
import dev.reins.android.design.Toggle
import dev.reins.android.design.pressable
import dev.reins.android.design.serviceName
import dev.reins.android.feedback.Event
import dev.reins.android.feedback.LocalFeedback
import dev.reins.android.feedback.play
import dev.reins.android.ui.common.untrusted
import dev.reins.core.EnvReleaseView
import dev.reins.core.EnvStoreView
import dev.reins.core.SecretsStoreView
import dev.reins.core.TokenStoreView

// Reins Secrets on the approval screen: an environment released for a command, values saved from the computer, a git
// host's token handed to the phone. Names only: a view never carries a value.

/** The most variable names shown on the main page; the rest are a tap away. */
private const val SHOWN_VARIABLES = 8

/**
 * A project's environment for one command on the computer: who runs it, the command in full (never cut), a warning
 * when the program is not the usual one, the project and environment, the variables' names, the folder, how long the
 * desktop app keeps the values (or that one is printed), and what the computer said it is for.
 */
@Composable
internal fun EnvReleaseFacts(r: EnvReleaseView, ui: ApprovalUi, viewModel: ApprovalViewModel) {
    val c = LocalColors.current
    val warning = r.programWarning?.let(::untrusted)?.takeIf { it.isNotBlank() }
    FactGroup(
        tag = "envRelease",
        borderTint = if (warning != null) c.warning.copy(alpha = 0.4f) else null,
        rows = buildList<@Composable () -> Unit> {
            add {
                FactRow("By", untrusted(r.runnerLabel), valueTag = "envRunner") {
                    Icon(
                        if (r.agent) Icons.Rounded.SmartToy else Icons.Rounded.Person,
                        if (r.agent) "Agent" else "Person",
                        Modifier.size(18.dp).testTag(if (r.agent) "envAgent" else "envPerson"),
                        tint = if (r.agent) c.accent else c.tertiary,
                    )
                }
            }
            add {
                Column(Modifier.fillMaxWidth().padding(horizontal = 18.dp, vertical = 13.dp)) {
                    RText("Command", RType.sans(13f, FontWeight.Medium), c.tertiary, maxLines = 1)
                    // In full, however long: it is what runs with the values.
                    RText(untrusted(r.command), RType.mono(14f, FontWeight.Medium).copy(lineHeight = 20.sp), c.text, Modifier.padding(top = 6.dp).testTag("envCommand"), ltr = true)
                }
            }
            if (warning != null) add { ProgramWarning(warning, untrusted(r.program)) }
            add { FactRow("Env", "${untrusted(r.project)} · ${untrusted(r.env)}", valueTag = "envWhere") }
            add { Variables(r) }
            add { FactRow("Folder", untrusted(r.root), mono = true, valueTag = "envRoot", maxLines = 2) }
            add {
                if (r.print) {
                    FactRow("Value", "Printed", valueTag = "envLease", valueColor = c.warning)
                } else {
                    FactRow("Kept", leaseLabel(r.leaseSecs).removePrefix("for "), valueTag = "envLease")
                }
            }
            r.purpose?.let(::untrusted)?.takeIf { it.isNotBlank() }?.let { add { FactRow("Says", "“$it”", valueTag = "envPurpose", maxLines = 3) } }
        },
    )
    if (!ui.readOnly && !r.print && r.commands.isNotEmpty()) AlwaysAsk(r, ui, viewModel)
}

/** The program is not where it usually is: the warning, its (?), and its whole path. */
@Composable
private fun ProgramWarning(warning: String, program: String) {
    val c = LocalColors.current
    Column(Modifier.fillMaxWidth().padding(horizontal = 18.dp, vertical = 12.dp).testTag("envProgramWarning")) {
        Row(verticalAlignment = Alignment.CenterVertically) {
            WarnTag(warning, c.warning, Modifier.testTag("envWarning"))
            Spacer(Modifier.weight(1f))
            HelpDot("programWarning", "The program that would get the values is not the usual one. Check its path before you approve.", size = 28.dp)
        }
        RText(program, RType.mono(13f).copy(lineHeight = 18.sp), c.warning, Modifier.padding(top = 8.dp).testTag("envProgram"), ltr = true)
    }
}

/** The variables' names, a few as chips and the rest a tap away; "Every variable" when the whole environment is asked. */
@OptIn(ExperimentalLayoutApi::class)
@Composable
private fun Variables(r: EnvReleaseView) {
    val c = LocalColors.current
    val nav = LocalApprovalNav.current
    if (r.allVariables) {
        FactRow("Variables", "Every variable", valueTag = "envVariables", valueColor = c.warning)
        return
    }
    Column(Modifier.fillMaxWidth().padding(horizontal = 18.dp, vertical = 13.dp)) {
        Row(verticalAlignment = Alignment.CenterVertically) {
            RText("Variables", RType.sans(13f, FontWeight.Medium), c.tertiary, Modifier.weight(1f), maxLines = 1)
            RText(r.variables.size.toString(), RType.mono(13f, FontWeight.Medium), c.tertiary, Modifier.testTag("envVariableCount"), maxLines = 1)
        }
        FlowRow(
            Modifier.padding(top = 8.dp).testTag("envVariables"),
            horizontalArrangement = Arrangement.spacedBy(6.dp),
            verticalArrangement = Arrangement.spacedBy(6.dp),
        ) {
            r.variables.take(SHOWN_VARIABLES).forEach { name -> NameChip(untrusted(name), Modifier.testTag("envVar:$name")) }
            val rest = r.variables.size - SHOWN_VARIABLES
            if (rest > 0) {
                val all = r.variables.joinToString("\n") { untrusted(it) }
                RText(
                    "+$rest",
                    RType.mono(12.5f, FontWeight.SemiBold),
                    c.accent,
                    Modifier
                        .clip(RoundedCornerShape(8.dp))
                        .background(c.accent.copy(alpha = 0.12f))
                        .pressable(shape = RoundedCornerShape(8.dp), label = "All variables") { nav.push(ApprovalPage.Text("Variables", all, mono = true, tag = "envVariablesAll")) }
                        .padding(horizontal = 9.dp, vertical = 5.dp)
                        .testTag("envVariablesMore"),
                    maxLines = 1,
                )
            }
        }
    }
}

@Composable
private fun NameChip(name: String, modifier: Modifier = Modifier) {
    val c = LocalColors.current
    RText(
        name,
        RType.mono(12.5f, FontWeight.Medium),
        c.text,
        modifier.clip(RoundedCornerShape(8.dp)).background(Surfaces.sunken(c)).padding(horizontal = 9.dp, vertical = 5.dp),
        maxLines = 1,
        ltr = true,
    )
}

/** "Ask every time" for this environment, saved at once. */
@Composable
private fun AlwaysAsk(r: EnvReleaseView, ui: ApprovalUi, viewModel: ApprovalViewModel) {
    val c = LocalColors.current
    val on = ui.draft.envAlwaysAsk ?: r.alwaysAsk
    FactGroup(
        tag = "envAlwaysAsk",
        rows = listOf {
            Row(Modifier.fillMaxWidth().defaultMinSize(minHeight = 54.dp).padding(start = 18.dp, end = 14.dp, top = 8.dp, bottom = 8.dp), verticalAlignment = Alignment.CenterVertically) {
                RText("Ask every time", RType.sans(15.5f, FontWeight.Medium), c.text, Modifier.weight(1f), maxLines = 1)
                HelpDot("alwaysAsk", "Every run with ${untrusted(r.project)} · ${untrusted(r.env)} waits for you. Nothing is remembered.", size = 28.dp)
                Spacer(Modifier.width(10.dp))
                Toggle(on, Modifier.testTag("alwaysAskToggle")) { viewModel.setEnvAlwaysAsk(it) }
            }
        },
    )
}

// ---- values from the computer --------------------------------------------------------------------------------------

/** The four check digits the terminal shows too, large, with a (?) that says to compare them. */
@Composable
internal fun CheckDigits(check: String, tag: String) {
    val c = LocalColors.current
    FactGroup(
        borderTint = c.accent.copy(alpha = 0.35f),
        rows = listOf {
            Column(Modifier.fillMaxWidth().padding(horizontal = 18.dp, vertical = 14.dp), horizontalAlignment = Alignment.CenterHorizontally) {
                Row(verticalAlignment = Alignment.CenterVertically) {
                    RText("Check digits", RType.sans(13.5f, FontWeight.Medium), c.tertiary, maxLines = 1)
                    Spacer(Modifier.width(6.dp))
                    HelpDot("check:$tag", "Compare with the terminal. Deny if they differ.", size = 26.dp)
                }
                Row(
                    Modifier.padding(top = 10.dp).testTag(tag).semantics(mergeDescendants = true) { contentDescription = check },
                    horizontalArrangement = Arrangement.spacedBy(8.dp),
                ) {
                    check.forEach { digit ->
                        Box(
                            Modifier.size(width = 50.dp, height = 60.dp).clip(RoundedCornerShape(14.dp)).background(Surfaces.sunken(c)),
                            contentAlignment = Alignment.Center,
                        ) { RText(digit.toString(), RType.mono(30f, FontWeight.SemiBold), c.text, maxLines = 1, align = TextAlign.Center) }
                    }
                }
            }
        },
    )
}

/** Values sent from the computer to the vault, in one approval: the check digits, then each environment, SSH key and token. */
@Composable
internal fun SecretsStoreFacts(store: SecretsStoreView) {
    CheckDigits(store.check, "storeCheck")
    if (store.envs.isNotEmpty()) {
        FactGroup(tag = "storeEnvs", rows = store.envs.mapIndexed { i, e -> { StoreEnvRow(i, e) } })
    }
    if (store.ssh.isNotEmpty()) {
        GroupHeader("SSH keys")
        FactGroup(
            tag = "storeSsh",
            rows = store.ssh.mapIndexed { i, key ->
                {
                    val c = LocalColors.current
                    Row(Modifier.fillMaxWidth().padding(horizontal = 18.dp, vertical = 13.dp), verticalAlignment = Alignment.CenterVertically) {
                        Icon(Icons.Rounded.Key, null, Modifier.size(17.dp), tint = c.accent)
                        Spacer(Modifier.width(12.dp))
                        Column(Modifier.weight(1f)) {
                            RText(untrusted(key.name), RType.sans(15.5f, FontWeight.Medium), c.text, Modifier.testTag("storeSsh:$i"), maxLines = 2)
                            RText(untrusted(key.fingerprint), RType.mono(12.5f).copy(lineHeight = 17.sp), c.tertiary, Modifier.padding(top = 2.dp).testTag("storeSshFp:$i"), ltr = true)
                        }
                    }
                }
            },
        )
    }
    if (store.tokens.isNotEmpty()) {
        GroupHeader("Tokens")
        FactGroup(tag = "storeTokens", rows = store.tokens.mapIndexed { i, t -> { TokenRow(t, "storeToken:$i") } })
    }
}

/** One environment: its name, NEW when it is made, what changes in counts; a tap shows the names. */
@OptIn(ExperimentalLayoutApi::class)
@Composable
private fun StoreEnvRow(i: Int, e: EnvStoreView) {
    val c = LocalColors.current
    val feedback = LocalFeedback.current
    var open by rememberSaveable(i) { mutableStateOf(false) }
    val turn by animateFloatAsState(if (open) 90f else 0f, label = "storeEnv")
    Column(
        Modifier
            .fillMaxWidth()
            .pressable(highlight = c.controlFill, shape = RoundedCornerShape(0.dp), label = "Names") {
                open = !open
                feedback.play(Event.expand(open))
            }
            .padding(horizontal = 18.dp, vertical = 13.dp)
            .testTag("storeEnv:$i"),
    ) {
        Row(verticalAlignment = Alignment.CenterVertically) {
            Row(Modifier.weight(1f), verticalAlignment = Alignment.CenterVertically) {
                RText("${untrusted(e.project)} · ${untrusted(e.env)}", RType.sans(15.5f, FontWeight.SemiBold), c.text, Modifier.weight(1f, fill = false).testTag("storeEnvName:$i"), maxLines = 1)
                if (e.newEnv) {
                    Spacer(Modifier.width(8.dp))
                    RText(
                        "New",
                        RType.sans(11.5f, FontWeight.SemiBold),
                        c.accent,
                        Modifier.clip(CircleShape).background(c.accent.copy(alpha = 0.14f)).padding(horizontal = 8.dp, vertical = 3.dp).testTag("storeNew:$i"),
                        maxLines = 1,
                    )
                }
            }
            Spacer(Modifier.width(8.dp))
            Icon(Icons.Rounded.ChevronRight, null, Modifier.size(18.dp).rotate(turn), tint = c.tertiary)
        }
        FlowRow(Modifier.padding(top = 8.dp), horizontalArrangement = Arrangement.spacedBy(6.dp), verticalArrangement = Arrangement.spacedBy(6.dp)) {
            CountPill(Icons.Rounded.Add, e.added.size, c.success, "added", "storeCount:$i:added")
            CountPill(Icons.Rounded.Sync, e.replaced.size, c.warning, "replaced", "storeCount:$i:replaced")
            CountPill(Icons.Rounded.Remove, e.removed.size, c.danger, "removed", "storeCount:$i:removed")
            CountPill(Icons.Rounded.DriveFileRenameOutline, e.renamed.size, c.accent, "renamed", "storeCount:$i:renamed")
            CountPill(Icons.Rounded.ContentCopy, e.copied.toInt(), c.secondary, "copied", "storeCount:$i:copied")
        }
        AnimatedVisibility(open, enter = expandVertically() + fadeIn(), exit = shrinkVertically() + fadeOut()) {
            Column(Modifier.padding(top = 10.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                Names("Added", e.added, "storeNames:$i:added")
                Names("Replaced", e.replaced, "storeNames:$i:replaced")
                Names("Removed", e.removed, "storeNames:$i:removed")
                Names("Renamed", e.renamed, "storeNames:$i:renamed")
                val from = e.copiedFrom?.let(::untrusted)?.takeIf { it.isNotBlank() }
                if (from != null && e.copied > 0u) {
                    RText("Copied ${e.copied} from $from", RType.sans(12.5f, FontWeight.Medium), c.tertiary, Modifier.testTag("storeCopied:$i"), maxLines = 2)
                }
            }
        }
    }
}

/** "+3" with its icon, in the colour of what happens; nothing when none. */
@Composable
private fun CountPill(icon: ImageVector, n: Int, tint: Color, what: String, tag: String) {
    if (n <= 0) return
    Row(
        Modifier
            .clip(CircleShape)
            .background(tint.copy(alpha = 0.12f))
            .padding(start = 7.dp, end = 9.dp, top = 4.dp, bottom = 4.dp)
            .semantics(mergeDescendants = true) { contentDescription = "$n $what" }
            .testTag(tag),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Icon(icon, null, Modifier.size(13.dp), tint = tint)
        Spacer(Modifier.width(4.dp))
        RText(n.toString(), RType.mono(12.5f, FontWeight.SemiBold), tint, maxLines = 1)
    }
}

@Composable
private fun Names(label: String, names: List<String>, tag: String) {
    if (names.isEmpty()) return
    val c = LocalColors.current
    Column {
        RText(label, RType.sans(12.5f, FontWeight.Medium), c.tertiary, maxLines = 1)
        RText(names.joinToString(", ") { untrusted(it) }, RType.mono(13f).copy(lineHeight = 18.sp), c.text, Modifier.padding(top = 2.dp).testTag(tag), ltr = true)
    }
}

/** A git host's token: the host's logo and name, the host, the scopes the computer reported, its check digits. */
@Composable
private fun TokenRow(t: TokenStoreView, tag: String, avatar: Dp = 34.dp) {
    val c = LocalColors.current
    Row(Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 12.dp).testTag(tag), verticalAlignment = Alignment.CenterVertically) {
        ServiceAvatar(t.service, size = avatar)
        Spacer(Modifier.width(12.dp))
        Column(Modifier.weight(1f)) {
            RText(serviceName(t.service), RType.sans(15.5f, FontWeight.SemiBold), c.text, maxLines = 1)
            RText(untrusted(t.host), RType.mono(12.5f), c.tertiary, Modifier.padding(top = 1.dp).testTag("$tag:host"), maxLines = 1, ltr = true)
            if (t.scopes.isNotEmpty()) {
                RText(t.scopes.joinToString(", ") { untrusted(it) }, RType.mono(12f).copy(lineHeight = 16.sp), c.secondary, Modifier.padding(top = 3.dp).testTag("$tag:scopes"), maxLines = 3, ltr = true)
            }
        }
        if (t.check.isNotBlank()) {
            Spacer(Modifier.width(10.dp))
            RText(untrusted(t.check), RType.mono(15f, FontWeight.SemiBold), c.text, Modifier.testTag("$tag:check"), maxLines = 1)
        }
    }
}

/** One git host's token handed to the phone's integration: its check digits, then the host and its scopes. */
@Composable
internal fun TokenStoreFacts(t: TokenStoreView) {
    if (t.check.isNotBlank()) CheckDigits(t.check, "tokenCheck")
    FactGroup(tag = "tokenStore", rows = listOf { TokenRow(t.copy(check = ""), "token", avatar = 38.dp) })
}
