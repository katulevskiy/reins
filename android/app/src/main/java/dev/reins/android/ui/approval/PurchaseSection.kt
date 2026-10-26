package dev.reins.android.ui.approval

import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.ui.semantics.semantics
import dev.reins.android.design.Glyph
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import dev.reins.android.design.Banner
import dev.reins.android.design.BannerKind
import dev.reins.android.design.Card
import dev.reins.android.design.CheckMark
import dev.reins.android.design.GlyphIcon
import dev.reins.android.design.Hairline
import dev.reins.android.design.LocalColors
import dev.reins.android.design.RText
import dev.reins.android.design.RTextField
import dev.reins.android.design.RType
import dev.reins.android.design.SelectChip
import dev.reins.android.design.SwitchRow
import dev.reins.android.design.Tag
import dev.reins.android.design.pressable
import dev.reins.android.ui.common.untrusted
import dev.reins.android.ui.payments.methodGlyph
import dev.reins.android.ui.payments.methodLine
import dev.reins.core.PurchaseView

/**
 * A purchase, laid out like a receipt: the store and the domain it is on, every line of the cart, the totals, then
 * where it goes and what pays (both can be changed here), what the AI said, and a spend limit for more like it.
 * Everything the AI wrote (the store's name, the items, the note) is shown as untrusted text.
 */
@Composable
fun PurchaseSection(p: PurchaseView, draft: PurchaseDraft, viewModel: ApprovalViewModel) {
    val c = LocalColors.current
    // What the store or Reins flagged (a first purchase here, a price that changed) comes first, in full.
    if (p.warnings.isNotEmpty()) {
        Column(Modifier.padding(horizontal = 22.dp).padding(top = 4.dp, bottom = 6.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
            for (w in p.warnings) WarnLine(untrusted(w), Modifier.testTag("purchaseWarning"))
        }
    }
    // The receipt (And-Pay 01): the store, each line, shipping and tax, and the total in serif, because it is the point.
    Column(
        Modifier
            .padding(horizontal = 18.dp, vertical = 6.dp)
            .fillMaxWidth()
            .clip(RoundedCornerShape(20.dp))
            .background(c.paperRaised)
            .border(1.dp, c.rule, RoundedCornerShape(20.dp))
            .testTag("receipt")
            .padding(horizontal = 16.dp, vertical = 14.dp),
    ) {
        Row(verticalAlignment = Alignment.CenterVertically) {
            Row(Modifier.weight(1f), verticalAlignment = Alignment.CenterVertically) {
                RText(untrusted(p.merchant), RType.sans(14.5f, FontWeight.Medium), c.ink, Modifier.weight(1f, fill = false), maxLines = 1)
                RText(" · ", RType.sans(14.5f), c.inkMeta, maxLines = 1)
                RText(p.domain, RType.mono(13f), c.inkMeta, Modifier.weight(1f, fill = false).testTag("domain"), maxLines = 1, ltr = true)
            }
            RText(p.currency, RType.mono(12f), c.inkMeta, Modifier.padding(start = 8.dp), maxLines = 1)
        }
        Spacer(Modifier.height(8.dp))
        for (item in p.items) {
            Row(Modifier.fillMaxWidth().padding(top = 6.dp), verticalAlignment = Alignment.Top) {
                Column(Modifier.weight(1f)) {
                    RText("${item.quantity} × ${untrusted(item.name)}", RType.sans(14.5f), c.ink, maxLines = 3)
                    val each = if (item.quantity > 1u) "${item.unitPrice} each" else null
                    listOfNotNull(item.details?.let(::untrusted), each).joinToString(" · ").takeIf { it.isNotEmpty() }?.let {
                        RText(it, RType.meta, c.inkMeta, Modifier.padding(top = 1.dp), maxLines = 2)
                    }
                }
                Spacer(Modifier.width(10.dp))
                RText(item.lineTotal, RType.mono(14f), c.ink, align = TextAlign.End, ltr = true)
            }
        }
        // "Shipping · Tax" on one line, as the receipt board has it; a discount on its own.
        val extras = listOfNotNull(p.shipping?.let { "Shipping" to it }, p.tax?.let { "Tax" to it })
        if (extras.isNotEmpty()) ExtraLine(extras.joinToString(" · ") { it.first }, extras.joinToString(" · ") { it.second })
        p.discount?.let { ExtraLine("Discount", "-$it") }
        Spacer(Modifier.height(12.dp))
        Hairline(inset = 0.dp)
        Row(Modifier.fillMaxWidth().padding(top = 10.dp), verticalAlignment = Alignment.CenterVertically) {
            RText("Total", RType.sans(15f, FontWeight.Medium), c.ink, Modifier.weight(1f))
            RText(p.total, RType.serif(28f, 32f), c.ink, Modifier.testTag("total"), maxLines = 1, ltr = true)
        }
    }
    p.note?.let {
        FactGroup(rows = listOf({ FactRow("The AI says", "“${untrusted(it)}”", maxLines = 6) }))
    }
    if (p.ships) {
        Section("Ship to")
        if (p.addresses.isEmpty()) {
            Banner("There is no address in your vault. Add an identity with an address there, then come back.", Modifier.padding(horizontal = 16.dp), BannerKind.Warning)
        }
        Picker {
            p.addresses.forEachIndexed { i, a ->
                if (i > 0) Hairline(inset = 52.dp)
                Choice(
                    picked = draft.addressId == a.id,
                    title = a.label,
                    detail = a.lines.joinToString(", "),
                    tag = "shipTo:${a.id}",
                ) { viewModel.editPurchase { it.copy(addressId = a.id) } }
            }
        }
    }
    Section("Pay with")
    Picker {
        p.methods.forEachIndexed { i, m ->
            if (i > 0) Hairline(inset = 52.dp)
            Choice(
                picked = draft.methodId == m.id,
                title = m.name,
                detail = m.unavailable ?: if (m.kind == "virtual_card" || m.kind == "pay_on_phone" || m.kind == "merchant_account") m.detail else methodLine(m),
                tag = "payWith:${m.id}",
                enabled = m.unavailable == null,
                glyph = { GlyphIcon(methodGlyph(m.kind), if (m.unavailable == null) c.accent else c.tertiary, size = 20.dp) },
            ) { viewModel.editPurchase { it.copy(methodId = m.id, limit = it.limit && m.id in p.limitMethods) } }
        }
    }
    val picked = p.methods.firstOrNull { it.id == draft.methodId }
    if (picked?.kind == "card") {
        RText(
            "A card from your vault is handed over in full: its number, expiry and code. It cannot be taken back, so prefer a virtual card when you can.",
            RType.sans(13f),
            c.secondary,
            Modifier.padding(horizontal = 24.dp, vertical = 6.dp),
        )
    }
    if (picked?.kind == "pay_on_phone") {
        RText(
            "Approving opens ${p.checkoutHost} here, and you pay there yourself. The AI gets nothing to pay with.",
            RType.sans(13f),
            c.secondary,
            Modifier.padding(horizontal = 24.dp, vertical = 6.dp),
        )
    }
    // Where card details go: sealed to the desktop app (by the name this phone kept), or through the server.
    if (picked?.kind == "card" || picked?.kind == "virtual_card") {
        p.delivery?.let {
            RText(it, RType.sans(13f), c.secondary, Modifier.padding(horizontal = 24.dp, vertical = 6.dp).testTag("delivery"))
        }
    }
    for (line in p.budgetLines) {
        RText(line, RType.sans(13f), c.tertiary, Modifier.padding(horizontal = 24.dp, vertical = 2.dp))
    }
    if (picked != null && picked.id in p.limitMethods) {
        Spacer(Modifier.height(6.dp))
        Card(Modifier.padding(horizontal = 16.dp, vertical = 6.dp)) {
            SwitchRow(
                "Approve more like this on their own",
                draft.limit,
                { on -> viewModel.editPurchase { it.copy(limit = on) } },
                subtitle = "A spend limit for this AI at ${p.domain}, with a virtual card. Anything above it still asks.",
                tag = "makeLimit",
            )
            if (draft.limit) {
                Column(Modifier.padding(horizontal = 16.dp).padding(bottom = 14.dp)) {
                    Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                        Column(Modifier.weight(1f)) {
                            FieldLabel("Up to, a purchase (${p.currency})")
                            RTextField(draft.limitPerPurchase, { v -> viewModel.editPurchase { it.copy(limitPerPurchase = v) } }, "25", tag = "limitEach", keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Decimal))
                        }
                        Column(Modifier.weight(1f)) {
                            FieldLabel("In a day (${p.currency})")
                            RTextField(draft.limitPerDay, { v -> viewModel.editPurchase { it.copy(limitPerDay = v) } }, "50", tag = "limitDay", keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Decimal))
                        }
                    }
                    Spacer(Modifier.height(10.dp))
                    Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                        for (d in listOf(1, 7, 30)) SelectChip(if (d == 1) "1 day" else "$d days", draft.limitDays == d) { viewModel.editPurchase { it.copy(limitDays = d) } }
                    }
                }
            }
        }
    }
}

/** A quiet line of the receipt: what on the left, the amounts in mono on the right. */
@Composable
private fun ExtraLine(label: String, value: String) {
    val c = LocalColors.current
    Row(Modifier.fillMaxWidth().padding(top = 8.dp)) {
        RText(label, RType.meta, c.inkMeta, Modifier.weight(1f), maxLines = 1)
        Spacer(Modifier.width(10.dp))
        RText(value, RType.mono(12.5f), c.inkMeta, maxLines = 1, align = TextAlign.End, ltr = true)
    }
}

/** A warning line on paper: the amber sign and the words, never cut. */
@Composable
internal fun WarnLine(text: String, modifier: Modifier = Modifier) {
    val c = LocalColors.current
    Row(modifier.fillMaxWidth().semantics(mergeDescendants = true) {}, horizontalArrangement = Arrangement.spacedBy(10.dp)) {
        GlyphIcon(Glyph.Warning, c.warning, size = 18.dp, modifier = Modifier.padding(top = 1.dp))
        RText(text, RType.sans(14.5f, lineHeight = 20f), c.ink, Modifier.weight(1f))
    }
}

@Composable
internal fun FieldLabel(text: String) {
    RText(text, RType.sans(12.5f, FontWeight.Medium), LocalColors.current.secondary, Modifier.padding(start = 4.dp, bottom = 5.dp), maxLines = 1)
}

@Composable
private fun Section(title: String) {
    val c = LocalColors.current
    RText(title.uppercase(), RType.sans(12.5f, FontWeight.Medium), c.secondary, Modifier.padding(start = 32.dp, top = 18.dp, bottom = 8.dp))
}

@Composable
private fun Picker(content: @Composable () -> Unit) {
    val c = LocalColors.current
    Column(Modifier.padding(horizontal = 16.dp).fillMaxWidth().clip(RoundedCornerShape(18.dp)).background(c.elevated)) { content() }
}

@Composable
private fun Choice(
    picked: Boolean,
    title: String,
    detail: String,
    tag: String,
    enabled: Boolean = true,
    glyph: (@Composable () -> Unit)? = null,
    onPick: () -> Unit,
) {
    val c = LocalColors.current
    Row(
        Modifier
            .fillMaxWidth()
            .testTag(tag)
            .graphicsLayer { alpha = if (enabled) 1f else 0.5f }
            .pressable(enabled = enabled, onClick = onPick)
            .padding(horizontal = 16.dp, vertical = 12.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        CheckMark(picked, enabled)
        Spacer(Modifier.width(12.dp))
        Column(Modifier.weight(1f)) {
            RText(title, RType.sans(15.5f, FontWeight.Medium), c.text, maxLines = 1)
            RText(detail, RType.sans(13f), if (enabled) c.secondary else c.warning, Modifier.padding(top = 2.dp), maxLines = 3)
        }
        if (glyph != null) {
            Spacer(Modifier.width(10.dp))
            glyph()
        }
    }
}
