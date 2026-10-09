package dev.reins.android.ui.payments

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import dev.reins.android.design.Banner
import dev.reins.android.design.BannerKind
import dev.reins.android.design.ButtonStyle
import dev.reins.android.design.CapsuleButton
import dev.reins.android.design.Card
import dev.reins.android.design.EmptyState
import dev.reins.android.design.Glyph
import dev.reins.android.design.GlyphIcon
import dev.reins.android.design.Group
import dev.reins.android.design.Hairline
import dev.reins.android.design.LocalColors
import dev.reins.android.design.RText
import dev.reins.android.design.RType
import dev.reins.android.design.Screen
import dev.reins.android.design.Spinner
import dev.reins.android.design.pressable
import dev.reins.android.platform.Browser
import dev.reins.android.ui.common.formatTime
import dev.reins.android.ui.common.untrusted
import dev.reins.core.PurchaseRecordView
import dev.reins.core.SpendTotal

/** Payments > Spending: what was spent this month, by which AI, and every purchase with what happened to it. */
@Composable
fun SpendingScreen(viewModel: PaymentsViewModel, onBack: () -> Unit) {
    val c = LocalColors.current
    val ui by viewModel.ui.collectAsStateWithLifecycle()
    LaunchedEffect(Unit) { viewModel.loadSpending() }
    Screen(title = "Spending", onBack = onBack) {
        ui.error?.let { Banner(it, Modifier.padding(horizontal = 16.dp, vertical = 8.dp), BannerKind.Error) }
        val spending = ui.spending
        if (spending == null) {
            Column(Modifier.fillMaxWidth().padding(32.dp), horizontalAlignment = Alignment.CenterHorizontally) { Spinner(c.secondary, size = 32.dp) }
            return@Screen
        }
        Card(Modifier.padding(16.dp)) {
            Column(Modifier.padding(20.dp).testTag("spentThisMonth")) {
                RText("This month", RType.sans(13f, FontWeight.Medium), c.secondary)
                Spacer(Modifier.height(4.dp))
                RText(totalsText(spending.totals), RType.sans(34f, FontWeight.SemiBold), c.text)
                for (ai in spending.byAi) {
                    Row(Modifier.fillMaxWidth().padding(top = 8.dp), verticalAlignment = Alignment.CenterVertically) {
                        RText(ai.connectionLabel, RType.sans(15f), c.secondary, Modifier.weight(1f), maxLines = 1)
                        RText(totalsText(ai.totals), RType.sans(15f, FontWeight.Medium), c.text)
                    }
                }
            }
        }
        if (spending.purchases.isEmpty()) {
            EmptyState(Glyph.Bag, "No purchases yet", "What your AIs buy, and what you approve for them, shows here.", tag = "noPurchases")
            return@Screen
        }
        Group(header = "Purchases") {
            spending.purchases.forEachIndexed { i, p ->
                if (i > 0) Hairline()
                PurchaseRow(p, ui.busy, viewModel)
            }
        }
        Spacer(Modifier.height(24.dp))
    }
}

private fun totalsText(totals: List<SpendTotal>): String =
    totals.filter { it.minor > 0 }.joinToString(" + ") { it.text }.ifEmpty { Money.format(0, "USD") }

@Composable
private fun PurchaseRow(p: PurchaseRecordView, busy: Boolean, viewModel: PaymentsViewModel) {
    val c = LocalColors.current
    val context = LocalContext.current
    var open by rememberSaveable(p.id) { mutableStateOf(p.mismatch != null) }
    Column(Modifier.fillMaxWidth().testTag("purchase:${p.id}")) {
        Row(
            Modifier.fillMaxWidth().pressable { open = !open }.padding(horizontal = 16.dp, vertical = 12.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            GlyphIcon(methodGlyph(p.methodKind), if (p.mismatch != null) c.danger else c.secondary, size = 21.dp)
            Spacer(Modifier.width(14.dp))
            Column(Modifier.weight(1f)) {
                RText(untrusted(p.merchant), RType.sans(16f, FontWeight.Medium), c.text, maxLines = 1)
                RText("${untrusted(p.connectionLabel)} · ${formatTime(p.at)}", RType.sans(13f), c.secondary, Modifier.padding(top = 2.dp), maxLines = 1)
            }
            Column(horizontalAlignment = Alignment.End) {
                RText(p.chargedText ?: p.totalText, RType.sans(16f, FontWeight.SemiBold), c.text)
                RText(statusWord(p), RType.sans(12.5f), statusColor(p), Modifier.padding(top = 2.dp))
            }
        }
        if (!open) return@Column
        Column(Modifier.padding(start = 51.dp, end = 16.dp, bottom = 14.dp), verticalArrangement = Arrangement.spacedBy(4.dp)) {
            p.mismatch?.let {
                Banner(
                    "The card made for ${p.domain} was charged by ${untrusted(it)}. It was closed, and spend limits for ${untrusted(p.connectionLabel)} wait until you have looked at this.",
                    Modifier.padding(bottom = 6.dp),
                    BannerKind.Warning,
                    tag = "mismatch:${p.id}",
                )
            }
            for (line in p.lines) RText(untrusted(line), RType.sans(14f), c.text)
            Detail("Store", p.domain)
            Detail("Paid with", untrusted(p.methodLabel))
            p.shipTo?.let { Detail("Shipped to", untrusted(it)) }
            Detail("Approved", if (p.byLimit) "by your spend limit" else "by you")
            p.orderId?.let { Detail("Order", untrusted(it)) }
            if (p.chargedText != null && p.chargedText != p.totalText) Detail("Approved total", p.totalText)
            if (p.chargedBy.isNotEmpty()) Detail("Charged by", p.chargedBy.joinToString(", ") { untrusted(it) })
            p.reportNote?.let { Detail("The AI says", untrusted(it)) }
            // What it counts for in budgets and limits: an open card its cap, else what was charged or approved.
            Detail("Counts as", p.countedText)
            Row(Modifier.padding(top = 8.dp), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                // The core keeps a receipt only when it is a page of the store's own site; the button says where it goes.
                p.receiptUrl?.let { url ->
                    val host = android.net.Uri.parse(url).host ?: p.domain
                    CapsuleButton("Receipt on $host", compact = true, style = ButtonStyle.Secondary, glyph = Glyph.Link) { Browser.open(context, url) }
                }
                if (p.clearable) {
                    CapsuleButton("Nothing was charged", Modifier.testTag("clear:${p.id}"), compact = true, style = ButtonStyle.Secondary, enabled = !busy) {
                        viewModel.clear(p.id)
                    }
                }
                if (p.cardOpen) {
                    CapsuleButton("Close card •• ${p.cardLast4 ?: ""}", Modifier.testTag("closeCard:${p.id}"), compact = true, style = ButtonStyle.Destructive, enabled = !busy) {
                        viewModel.closeCard(p.id)
                    }
                }
                if (p.mismatch != null) {
                    CapsuleButton("I've seen it", Modifier.testTag("acknowledge:${p.id}"), compact = true, style = ButtonStyle.Secondary, enabled = !busy) {
                        viewModel.acknowledge(p.id)
                    }
                }
            }
        }
    }
}

@Composable
private fun Detail(label: String, value: String) {
    val c = LocalColors.current
    Row {
        RText(label, RType.sans(13.5f), c.secondary, Modifier.width(112.dp))
        RText(value, RType.sans(13.5f), c.text, Modifier.weight(1f), ltr = label == "Store")
    }
}

private fun statusWord(p: PurchaseRecordView): String = when {
    p.mismatch != null -> "Check this"
    p.status == "completed" -> "Ordered"
    p.status == "failed" -> "Failed"
    p.status == "cancelled" -> "Cancelled"
    p.methodKind == "pay_on_phone" -> "Handed to you"
    else -> "Approved"
}

@Composable
private fun statusColor(p: PurchaseRecordView) = LocalColors.current.let { c ->
    when {
        p.mismatch != null -> c.danger
        p.status == "completed" -> c.success
        p.status == "failed" || p.status == "cancelled" -> c.tertiary
        else -> c.secondary
    }
}
