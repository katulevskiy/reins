package dev.reins.android.ui.approval

import androidx.compose.foundation.background
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
    Card(Modifier.padding(horizontal = 16.dp, vertical = 8.dp).testTag("receipt")) {
        Column(Modifier.padding(18.dp)) {
            Row(verticalAlignment = Alignment.CenterVertically) {
                Column(Modifier.weight(1f)) {
                    RText(untrusted(p.merchant), RType.sans(20f, FontWeight.SemiBold), c.text, maxLines = 2)
                    RText(p.domain, RType.mono(13.5f), c.secondary, Modifier.padding(top = 2.dp).testTag("domain"), maxLines = 1, ltr = true)
                }
                Tag(p.currency, mono = true)
            }
            Spacer(Modifier.height(14.dp))
            Hairline(inset = 0.dp)
            for (item in p.items) {
                Row(Modifier.fillMaxWidth().padding(top = 10.dp), verticalAlignment = Alignment.Top) {
                    RText("${item.quantity} ×", RType.mono(14f), c.secondary, Modifier.width(44.dp))
                    Column(Modifier.weight(1f)) {
                        RText(untrusted(item.name), RType.sans(15f, FontWeight.Medium), c.text, maxLines = 3)
                        val each = if (item.quantity > 1u) "${item.unitPrice} each" else null
                        listOfNotNull(item.details?.let(::untrusted), each).joinToString(" · ").takeIf { it.isNotEmpty() }?.let {
                            RText(it, RType.sans(13f), c.secondary, Modifier.padding(top = 1.dp), maxLines = 2)
                        }
                    }
                    Spacer(Modifier.width(10.dp))
                    RText(item.lineTotal, RType.mono(14.5f), c.text, align = TextAlign.End)
                }
            }
            Spacer(Modifier.height(12.dp))
            Hairline(inset = 0.dp)
            Spacer(Modifier.height(6.dp))
            Amount("Items", p.subtotal)
            p.shipping?.let { Amount("Shipping", it) }
            p.tax?.let { Amount("Tax", it) }
            p.discount?.let { Amount("Discount", "-$it") }
            Row(Modifier.fillMaxWidth().padding(top = 8.dp), verticalAlignment = Alignment.CenterVertically) {
                RText("Total", RType.sans(17f, FontWeight.SemiBold), c.text, Modifier.weight(1f))
                RText(p.total, RType.mono(22f, FontWeight.SemiBold), c.text, Modifier.testTag("total"))
            }
        }
    }
    for (w in p.warnings) {
        Banner(untrusted(w), Modifier.padding(horizontal = 16.dp, vertical = 4.dp), BannerKind.Warning, tag = "purchaseWarning")
    }
    p.note?.let {
        Column(Modifier.padding(horizontal = 20.dp, vertical = 8.dp)) {
            RText("Why, in the AI's words", RType.sans(12.5f, FontWeight.Medium), c.secondary)
            RText("“${untrusted(it)}”", RType.sans(15f, lineHeight = 21f), c.text, Modifier.padding(top = 3.dp), maxLines = 6)
        }
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

@Composable
internal fun FieldLabel(text: String) {
    RText(text, RType.sans(12.5f, FontWeight.Medium), LocalColors.current.secondary, Modifier.padding(start = 4.dp, bottom = 5.dp), maxLines = 1)
}

@Composable
private fun Amount(label: String, value: String) {
    val c = LocalColors.current
    Row(Modifier.fillMaxWidth().padding(top = 4.dp)) {
        RText(label, RType.sans(14.5f), c.secondary, Modifier.weight(1f))
        RText(value, RType.mono(14.5f), c.text)
    }
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
