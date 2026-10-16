package dev.reins.android.ui.pairing

import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.rounded.People
import androidx.compose.material3.Icon
import androidx.compose.runtime.remember
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.selected
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import dev.reins.android.design.Banner
import dev.reins.android.design.BannerKind
import dev.reins.android.design.Field
import dev.reins.android.design.Glyph
import dev.reins.android.design.GlyphIcon
import dev.reins.android.design.LabelText
import dev.reins.android.design.LocalColors
import dev.reins.android.design.RText
import dev.reins.android.design.RTextField
import dev.reins.android.design.RType
import dev.reins.android.design.pressable
import dev.reins.android.feedback.Event
import dev.reins.android.feedback.feedbackAction
import dev.reins.android.platform.Authenticator
import dev.reins.android.ui.approval.AskButtons
import dev.reins.android.ui.approval.AskDock
import dev.reins.android.ui.approval.AskField
import dev.reins.android.ui.approval.AskLoading
import dev.reins.android.ui.approval.FactGroup
import dev.reins.android.ui.approval.FactRow
import dev.reins.android.ui.approval.FieldVerdict
import dev.reins.android.ui.common.quotedName
import dev.reins.android.ui.common.untrustedName

/**
 * Connecting a new AI or computer (Reins Signal, iOS-Connect 03): the teal pair field, then "Tap the number your
 * computer shows" with three big targets, the desktop app's key to compare, and the name. Connect stays at 40 % until
 * a number is tapped. [onClose] leaves it waiting ("not now").
 *
 * The other side chooses its own name, so the field never says it: the serif ask, the label and the colour are Reins'
 * own (a computer when it brought a key, an AI app otherwise), and the name only appears quoted on one short line. The
 * key's digits, which the server cannot fake past the comparison, sit in mono on a row of their own.
 */
@Composable
fun PairingSheet(viewModel: PairingViewModel, authenticator: Authenticator, onDone: () -> Unit, onClose: () -> Unit = onDone) {
    val c = LocalColors.current
    val ui by viewModel.ui.collectAsStateWithLifecycle()
    LaunchedEffect(ui.finished) { if (ui.finished) onDone() }
    val view = ui.view
    if (view == null) {
        AskLoading(ui.loading, ui.error, onClose)
        return
    }
    val name = quotedName(view.clientName)
    val computer = view.keyFingerprint != null
    Column(Modifier.fillMaxSize().background(c.paper)) {
        Column(Modifier.weight(1f).verticalScroll(rememberScrollState())) {
            AskField(
                Field.Pair,
                if (computer) "Computer · Pair" else "AI app · Pair",
                pairingAsk(computer),
                if (computer) "computer" else "AI app",
                onClose = onClose,
                closeGlyph = Glyph.Close,
                closeLabel = "Close",
                top = {
                    GlyphIcon(if (computer) Glyph.Laptop else Glyph.Apps, c.onField, size = 20.dp)
                    RText(if (computer) "New computer" else "New AI app", RType.sans(14.5f, FontWeight.Medium), c.onField, Modifier.weight(1f), maxLines = 1)
                },
                verdict = { FieldVerdict("Only if you just started this yourself.") },
            )
            // What the other side says of itself: quoted, one line, on rows of their own, never in the ask.
            FactGroup(
                Modifier.padding(top = 14.dp),
                tag = "pairWho",
                rows = listOf(
                    { FactRow("Calls itself", name, valueTag = "pairName") },
                    { FactRow("From", untrustedName(view.clientHost), mono = true, valueTag = "pairHost") },
                ),
            )
            view.keyFingerprint?.let { KeyRow(it) }
            Column(Modifier.padding(horizontal = 22.dp).padding(top = 22.dp)) {
                RText(
                    if (computer) "Tap the number your computer shows." else "Tap the number your browser shows.",
                    RType.sans(15f),
                    c.ink2,
                )
                Row(Modifier.fillMaxWidth().padding(top = 12.dp), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                    ui.codes.forEach { code ->
                        val label = "%02d".format(code)
                        val on = ui.chosen == code
                        Box(
                            Modifier
                                .weight(1f)
                                .height(64.dp)
                                .clip(RoundedCornerShape(18.dp))
                                .background(if (on) c.paperRaised else c.paperSunken)
                                .then(if (on) Modifier.border(2.dp, c.ink, RoundedCornerShape(18.dp)) else Modifier)
                                .pressable(shape = RoundedCornerShape(18.dp), label = label, onClick = feedbackAction(Event.Selection) { viewModel.choose(code) })
                                .semantics { selected = on }
                                .testTag("code:$label"),
                            contentAlignment = Alignment.Center,
                        ) { RText(label, RType.mono(26f), c.ink, maxLines = 1, ltr = true) }
                    }
                }
            }
            Column(Modifier.padding(horizontal = 22.dp)) {
                LabelText("Name it", c.inkMeta, Modifier.padding(top = 18.dp, bottom = 8.dp))
                val naming = remember { androidx.compose.ui.focus.FocusRequester() }
                if (ui.sameClient) {
                    // Two accounts of one AI look the same to Reins: the name is what tells them apart.
                    Row(Modifier.padding(bottom = 8.dp).testTag("pairSameClient"), verticalAlignment = Alignment.CenterVertically) {
                        Icon(Icons.Rounded.People, null, tint = c.accent, modifier = Modifier.size(18.dp))
                        RText(sameClientLine(view), RType.meta, c.accent, Modifier.padding(start = 8.dp))
                    }
                    LaunchedEffect(view.id) { runCatching { naming.requestFocus() } }
                }
                RTextField(ui.label, viewModel::setLabel, untrustedName(view.clientName, 48).ifEmpty { "Name" }, tag = "label", focusRequester = naming)
                val policy by viewModel.startingPolicy.collectAsStateWithLifecycle()
                dev.reins.android.ui.grants.StartingRuleText.onPairing(policy)?.let {
                    RText(it, RType.meta, c.inkMeta, Modifier.padding(top = 12.dp).testTag("startingRuleNote"))
                }
                ui.error?.let { Banner(it, Modifier.padding(top = 16.dp), BannerKind.Error) }
                if (ui.needsScreenLock) {
                    val context = androidx.compose.ui.platform.LocalContext.current
                    dev.reins.android.ui.common.ScreenLockCard(Modifier.padding(top = 16.dp)) { dev.reins.android.platform.ScreenLock.openSettings(context) }
                }
                Spacer(Modifier.height(20.dp))
            }
        }
        AskDock {
            AskButtons("Connect", onDeny = viewModel::deny, onApprove = { viewModel.approve(authenticator) }, busy = ui.busy, actionEnabled = ui.chosen != null)
        }
    }
}

/** The serif ask on the pair field: always Reins' words, never the name the other side sent. */
internal fun pairingAsk(computer: Boolean): String = if (computer) "Connect a computer" else "Connect an AI app"

/**
 * The Reins desktop app's key, as the digits the computer shows too. Matching them is what stops the server from
 * slipping in a key of its own, so they sit on their own row in mono, apart from anything the other side named.
 */
@Composable
private fun KeyRow(fingerprint: String) {
    val c = LocalColors.current
    Column(
        Modifier
            .padding(horizontal = 22.dp)
            .padding(top = 22.dp)
            .fillMaxWidth()
            .clip(RoundedCornerShape(18.dp))
            .background(c.paperRaised)
            .border(1.dp, c.rule, RoundedCornerShape(18.dp))
            .testTag("keyFingerprint")
            .semantics(mergeDescendants = true) {}
            .padding(horizontal = 18.dp, vertical = 16.dp),
        verticalArrangement = Arrangement.spacedBy(6.dp),
    ) {
        Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            GlyphIcon(Glyph.Key, c.ink2, size = 16.dp)
            LabelText("Desktop key", c.inkMeta)
        }
        RText(fingerprint, RType.mono(30f, FontWeight.Medium).copy(letterSpacing = 1.5.sp), c.ink, Modifier.testTag("keyDigits"), maxLines = 1, ltr = true)
        RText("This computer's key must match. If it differs, deny.", RType.meta, c.inkMeta, Modifier.testTag("keyMustMatch"))
    }
}
