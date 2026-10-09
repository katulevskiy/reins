package dev.reins.android.ui.payments

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ExperimentalLayoutApi
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import dev.reins.android.design.Banner
import dev.reins.android.design.BannerKind
import dev.reins.android.design.ButtonStyle
import dev.reins.android.design.CapsuleButton
import dev.reins.android.design.Card
import dev.reins.android.design.CheckMark
import dev.reins.android.design.ConfirmDialog
import dev.reins.android.design.Glyph
import dev.reins.android.design.GlyphIcon
import dev.reins.android.design.Group
import dev.reins.android.design.Hairline
import dev.reins.android.design.ListRow
import dev.reins.android.design.LocalColors
import dev.reins.android.design.RText
import dev.reins.android.design.RTextField
import dev.reins.android.design.RType
import dev.reins.android.design.Screen
import dev.reins.android.design.SelectChip
import dev.reins.android.design.ServiceAvatar
import dev.reins.android.design.Spinner
import dev.reins.android.design.SwitchRow
import dev.reins.android.design.Tag
import dev.reins.android.design.Toggle
import dev.reins.android.design.pressable
import dev.reins.android.state.AppState
import dev.reins.android.ui.approval.FieldLabel
import dev.reins.android.ui.common.formatDate
import dev.reins.core.BudgetView
import dev.reins.core.LimitPeriod
import dev.reins.core.PaymentMethodView
import dev.reins.core.PaymentsOverview
import dev.reins.core.SpendLimitInput

/** Integrations > Payments: what pays, where things go, and what an AI may spend. */
@Composable
fun PaymentsScreen(viewModel: PaymentsViewModel, state: AppState, onBack: () -> Unit, onSpending: () -> Unit) {
    val c = LocalColors.current
    val ui by viewModel.ui.collectAsStateWithLifecycle()
    // Cards and addresses come from the vault, which may have changed since the screen was last open.
    LaunchedEffect(Unit) { viewModel.refresh() }
    Screen(title = "Payments", onBack = onBack) {
        val overview = ui.overview
        if (overview == null) {
            Column(Modifier.fillMaxWidth().padding(32.dp), horizontalAlignment = Alignment.CenterHorizontally) {
                if (ui.loading) Spinner(c.secondary, size = 32.dp)
                ui.error?.let { Banner(it, Modifier.padding(top = 16.dp), BannerKind.Error) }
            }
            return@Screen
        }
        ui.error?.let { Banner(it, Modifier.padding(horizontal = 16.dp, vertical = 8.dp), BannerKind.Error, tag = "paymentsError") }
        ui.notice?.let { Banner(it, Modifier.padding(horizontal = 16.dp, vertical = 8.dp), tag = "paymentsNotice") }
        if (!overview.enabled) {
            Intro(ui.busy, onEnable = viewModel::enable)
            return@Screen
        }
        Group(Modifier.padding(top = 8.dp)) {
            ListRow(
                "Spending",
                Modifier.testTag("openSpending"),
                subtitle = "Every purchase, and what was spent this month",
                glyph = Glyph.Bag,
                chevron = true,
                onClick = onSpending,
            )
        }
        Methods(overview, ui.busy, viewModel)
        VirtualCards(overview, ui.busy, viewModel)
        Addresses(overview, viewModel)
        Limits(overview, state, ui.busy, viewModel)
        Budget(overview, ui.busy, viewModel)
        Group(
            header = "Payment key",
            footer = "Every approval carries a mandate signed by this key, so the cart you approved can be proved later. To pay from a computer through Reins, run reins payments-trust there and type at least the first 16 characters.",
        ) {
            RText(
                overview.mandateKey.chunked(4).joinToString(" "),
                RType.mono(15f),
                c.text,
                Modifier.padding(16.dp).testTag("paymentKey"),
                ltr = true,
            )
        }
        var confirmOff by remember { mutableStateOf(false) }
        Group {
            ListRow("Turn off Payments", Modifier.testTag("paymentsOff"), glyph = Glyph.SignOut, destructive = true) { confirmOff = true }
        }
        if (confirmOff) {
            ConfirmDialog(
                "Turn off Payments?",
                "Your AIs no longer see the payment tools, and the spend limits go. Open virtual cards are closed; the Privacy.com key is removed once none is left open. Your settings and the spending history stay.",
                "Turn off",
                onConfirm = {
                    confirmOff = false
                    viewModel.disable()
                },
                onDismiss = { confirmOff = false },
            )
        }
        Spacer(Modifier.height(24.dp))
    }
}

@Composable
private fun Intro(busy: Boolean, onEnable: () -> Unit) {
    val c = LocalColors.current
    Card(Modifier.padding(16.dp)) {
        Column(Modifier.padding(20.dp)) {
            ServiceAvatar("payments", size = 48.dp)
            Spacer(Modifier.height(14.dp))
            RText("Let your AIs buy things", RType.sans(20f, FontWeight.SemiBold), c.text)
            Spacer(Modifier.height(8.dp))
            RText(
                "An AI fills a cart and asks Reins to buy it. You see it here like a receipt (the store, every item, the total, where it goes and what pays) and approve it with your fingerprint or screen lock.",
                RType.sans(15f, lineHeight = 21f),
                c.secondary,
            )
            Spacer(Modifier.height(10.dp))
            for (line in listOf(
                "A virtual card made for that cart, capped at its total",
                "A card from your vault, for that purchase only",
                "The card the store already has, or you pay here yourself",
            )) {
                Row(Modifier.padding(vertical = 3.dp), verticalAlignment = Alignment.CenterVertically) {
                    GlyphIcon(Glyph.Check, c.success, size = 16.dp)
                    Spacer(Modifier.width(8.dp))
                    RText(line, RType.sans(14.5f), c.text)
                }
            }
            Spacer(Modifier.height(18.dp))
            CapsuleButton("Turn on Payments", Modifier.fillMaxWidth().testTag("paymentsEnable"), busy = busy, enabled = !busy, onClick = onEnable)
        }
    }
}

/** "Visa •• 4242 · 04/29". */
internal fun methodLine(m: PaymentMethodView): String = listOfNotNull(
    if (m.last4 != null) "${m.brand ?: "Card"} •• ${m.last4}" else null,
    m.expiry,
).joinToString(" · ").ifEmpty { m.detail }

@Composable
private fun Methods(overview: PaymentsOverview, busy: Boolean, viewModel: PaymentsViewModel) {
    val c = LocalColors.current
    Group(
        header = "Pay with",
        footer = "AIs see a method's name, kind and last four digits, never a card number. Tap one to make it the one picked first.",
    ) {
        overview.methods.forEachIndexed { i, m ->
            if (i > 0) Hairline(inset = 54.dp)
            val isDefault = overview.defaultMethod == m.id
            Row(
                Modifier
                    .fillMaxWidth()
                    .testTag("method:${m.id}")
                    .pressable(enabled = m.enabled && !busy) { viewModel.setDefaultMethod(if (isDefault) null else m.id) }
                    .padding(horizontal = 16.dp, vertical = 12.dp),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                GlyphIcon(methodGlyph(m.kind), if (m.enabled) c.accent else c.tertiary, size = 22.dp)
                Spacer(Modifier.width(16.dp))
                Column(Modifier.weight(1f)) {
                    Row(verticalAlignment = Alignment.CenterVertically) {
                        RText(m.name, RType.sans(16f, FontWeight.Medium), c.text, Modifier.weight(1f, fill = false), maxLines = 1)
                        if (isDefault) {
                            Spacer(Modifier.width(8.dp))
                            Tag("First choice", tint = c.accent)
                        }
                    }
                    RText(m.unavailable ?: methodLine(m), RType.sans(13f), if (m.unavailable != null) c.warning else c.secondary, Modifier.padding(top = 2.dp), maxLines = 2)
                }
                // The virtual card is switched on by connecting its provider, below.
                if (m.kind != "virtual_card") {
                    Spacer(Modifier.width(10.dp))
                    Toggle(m.enabled, Modifier.testTag("toggle:${m.id}"), enabled = !busy) { viewModel.setMethod(m.id, it) }
                }
            }
        }
        if (!overview.vaultReady) {
            Banner(
                "Cards and addresses come from your password vault. Connect it under Integrations to use them.",
                Modifier.padding(12.dp),
                BannerKind.Notice,
            )
        }
    }
}

internal fun methodGlyph(kind: String): Glyph = when (kind) {
    "virtual_card" -> Glyph.Shield
    "card" -> Glyph.Card
    "merchant_account" -> Glyph.Bag
    else -> Glyph.Phone
}

@Composable
private fun VirtualCards(overview: PaymentsOverview, busy: Boolean, viewModel: PaymentsViewModel) {
    val c = LocalColors.current
    val provider = overview.provider
    Group(
        header = "Virtual cards",
        footer = "Each purchase gets a new Privacy.com card, locked to the store and capped at the approved total plus the tolerance. The key stays on this phone.",
    ) {
        if (provider == null) {
            var open by rememberSaveable { mutableStateOf(false) }
            var key by rememberSaveable { mutableStateOf("") }
            var sandbox by rememberSaveable { mutableStateOf(false) }
            var singleUse by rememberSaveable { mutableStateOf(false) }
            ListRow(
                "Connect Privacy.com",
                Modifier.testTag("connectPrivacy"),
                subtitle = "Paste your API key from privacy.com (Account → API)",
                glyph = Glyph.Plus,
                tint = c.accent,
            ) { open = !open }
            if (open) {
                Column(Modifier.padding(horizontal = 16.dp).padding(bottom = 14.dp)) {
                    RTextField(key, { key = it }, "API key", tag = "privacyKey", mono = true, password = true)
                    SwitchRow("Sandbox", sandbox, { sandbox = it }, subtitle = "Test cards that charge nothing", tag = "privacySandbox")
                    SwitchRow("Single-use cards", singleUse, { singleUse = it }, subtitle = "Closed after the first charge. Off: locked to the store, so split shipments still go through.")
                    CapsuleButton(
                        "Connect",
                        Modifier.fillMaxWidth().testTag("privacyConnect"),
                        enabled = !busy && key.isNotBlank(),
                        busy = busy,
                    ) { viewModel.connectProvider(key, sandbox, singleUse) }
                }
            }
        } else {
            ListRow(
                provider.name + if (provider.sandbox) " (sandbox)" else "",
                subtitle = "Connected ${formatDate(provider.addedAt)}",
                glyph = Glyph.ShieldCheck,
                tint = c.success,
            )
            Hairline(inset = 54.dp)
            Column(Modifier.padding(horizontal = 16.dp, vertical = 12.dp)) {
                RText("Tolerance", RType.sans(16f, FontWeight.Medium), c.text)
                RText(
                    "How much more than the approved total a card allows, for shipping or tax that changes at checkout (at least 1.00).",
                    RType.sans(13f),
                    c.secondary,
                    Modifier.padding(top = 2.dp, bottom = 10.dp),
                )
                Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                    for (pct in listOf(0, 5, 10, 20)) {
                        SelectChip("$pct%", overview.tolerancePct.toInt() == pct, Modifier.testTag("tolerance:$pct")) {
                            viewModel.setCardOptions(pct, provider.singleUse)
                        }
                    }
                }
            }
            Hairline()
            SwitchRow("Single-use cards", provider.singleUse, { viewModel.setCardOptions(overview.tolerancePct.toInt(), it) }, subtitle = "Closed after the first charge")
            Hairline()
            ListRow("Disconnect", Modifier.testTag("disconnectPrivacy"), destructive = true) { viewModel.disconnectProvider() }
        }
    }
}

@Composable
private fun Addresses(overview: PaymentsOverview, viewModel: PaymentsViewModel) {
    val c = LocalColors.current
    Group(
        header = "Ship to",
        footer = "Identities in your vault that have an address. AIs see the label, the city and the country; the street only comes with a purchase you approved.",
    ) {
        if (overview.addresses.isEmpty()) {
            ListRow("No addresses yet", subtitle = "Add an identity with an address to your vault", glyph = Glyph.Info)
        }
        overview.addresses.forEachIndexed { i, a ->
            if (i > 0) Hairline(inset = 54.dp)
            val picked = overview.defaultAddress == a.id
            Row(
                Modifier
                    .fillMaxWidth()
                    .testTag("address:${a.id}")
                    .pressable { viewModel.setDefaultAddress(if (picked) null else a.id) }
                    .padding(horizontal = 16.dp, vertical = 12.dp),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                CheckMark(picked)
                Spacer(Modifier.width(14.dp))
                Column(Modifier.weight(1f)) {
                    RText(a.label, RType.sans(16f, FontWeight.Medium), c.text, maxLines = 1)
                    RText(a.lines.joinToString(", "), RType.sans(13f), c.secondary, Modifier.padding(top = 2.dp), maxLines = 2)
                }
            }
        }
    }
}

@OptIn(ExperimentalLayoutApi::class)
@Composable
private fun Limits(overview: PaymentsOverview, state: AppState, busy: Boolean, viewModel: PaymentsViewModel) {
    val c = LocalColors.current
    val connections by state.connections.collectAsStateWithLifecycle()
    var adding by rememberSaveable { mutableStateOf(false) }
    Group(
        header = "Spend limits",
        footer = "A limit lets one AI buy with a virtual card without asking you, within its amounts. Anything else, or anything above it, asks.",
    ) {
        overview.limits.forEachIndexed { i, l ->
            if (i > 0) Hairline()
            ListRow(
                l.summary,
                Modifier.testTag("limit:${l.id}"),
                subtitle = "${l.connectionLabel} · ${Money.format(l.spent, l.currency)} spent · " +
                    if (l.active) "until ${formatDate(l.expiresAt)}" else "ended",
                glyph = Glyph.Bolt,
                tint = if (l.active) c.accent else c.tertiary,
                trailing = {
                    GlyphIcon(Glyph.Trash, c.danger, size = 20.dp, modifier = Modifier.testTag("removeLimit:${l.id}").pressable { viewModel.removeLimit(l.id) })
                },
            )
        }
        if (overview.limits.isNotEmpty()) Hairline()
        if (overview.provider == null) {
            ListRow("Connect Privacy.com to add a limit", subtitle = "A limit always pays with a virtual card, whose cap the card enforces", glyph = Glyph.Info)
            return@Group
        }
        ListRow("Add a spend limit", Modifier.testTag("addLimit"), glyph = Glyph.Plus, tint = c.accent) { adding = !adding }
        if (!adding) return@Group
        var ai by rememberSaveable { mutableStateOf(connections.firstOrNull()?.id ?: "") }
        var stores by rememberSaveable { mutableStateOf("") }
        var perPurchase by rememberSaveable { mutableStateOf("25") }
        var perPeriod by rememberSaveable { mutableStateOf("50") }
        var period by rememberSaveable { mutableStateOf(LimitPeriod.DAY.name) }
        var days by rememberSaveable { mutableStateOf(7) }
        val currency = "USD"
        Column(Modifier.padding(horizontal = 16.dp).padding(bottom = 16.dp)) {
            RText("For", RType.sans(13f, FontWeight.Medium), c.secondary, Modifier.padding(bottom = 6.dp))
            FlowRow(horizontalArrangement = Arrangement.spacedBy(8.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                for (conn in connections) SelectChip(conn.label, ai == conn.id, Modifier.testTag("limitAi:${conn.id}")) { ai = conn.id }
            }
            Spacer(Modifier.height(12.dp))
            RTextField(stores, { stores = it }, "Stores (amazon.com, ebay.com), or empty for any", tag = "limitStores")
            Spacer(Modifier.height(8.dp))
            Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                Column(Modifier.weight(1f)) {
                    FieldLabel("Up to, a purchase ($currency)")
                    RTextField(perPurchase, { perPurchase = it }, "25", tag = "limitPerPurchase", keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Decimal))
                }
                Column(Modifier.weight(1f)) {
                    FieldLabel("In all, per period")
                    RTextField(perPeriod, { perPeriod = it }, "50", tag = "limitPerPeriod", keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Decimal))
                }
            }
            Spacer(Modifier.height(10.dp))
            Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                for (p in LimitPeriod.entries) SelectChip("a ${p.name.lowercase()}", period == p.name) { period = p.name }
            }
            Spacer(Modifier.height(10.dp))
            RText("Lasts", RType.sans(13f, FontWeight.Medium), c.secondary, Modifier.padding(bottom = 6.dp))
            Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                for (d in listOf(1, 7, 30, 90)) SelectChip(if (d == 1) "1 day" else "$d days", days == d) { days = d }
            }
            Spacer(Modifier.height(14.dp))
            val purchase = Money.parse(perPurchase, currency)
            val inAll = Money.parse(perPeriod, currency)
            val ok = ai.isNotEmpty() && purchase != null && inAll != null && inAll >= purchase
            if (!ok && (perPurchase.isNotEmpty() || perPeriod.isNotEmpty())) {
                RText("Enter amounts like 25 or 25.00; the total at least the amount a purchase.", RType.sans(13f), c.warning, Modifier.padding(bottom = 8.dp))
            }
            CapsuleButton("Add limit", Modifier.fillMaxWidth().testTag("limitSave"), enabled = ok && !busy, busy = busy) {
                viewModel.addLimit(
                    SpendLimitInput(
                        connectionId = ai,
                        merchants = stores.split(',', ' ').map { it.trim() }.filter { it.isNotEmpty() },
                        method = "virtual_card",
                        currency = currency,
                        perPurchase = purchase ?: 0,
                        perPeriod = inAll ?: 0,
                        period = LimitPeriod.valueOf(period),
                        durationSecs = days * 86_400L,
                    ),
                )
                adding = false
            }
        }
    }
}

@Composable
private fun Budget(overview: PaymentsOverview, busy: Boolean, viewModel: PaymentsViewModel) {
    val c = LocalColors.current
    val current = overview.budgets.firstOrNull { it.connectionId.isEmpty() }
    val currency = current?.currency ?: "USD"
    var perPurchase by rememberSaveable(current) { mutableStateOf(current?.perPurchase?.let { Money.plain(it, currency) } ?: "") }
    var perMonth by rememberSaveable(current) { mutableStateOf(current?.perMonth?.let { Money.plain(it, currency) } ?: "") }
    var stores by rememberSaveable(current) { mutableStateOf(current?.merchants?.joinToString(", ") ?: "") }
    Group(
        header = "Budget",
        footer = "For every AI together. A purchase over it is refused before it reaches you, and the AI is told why. Leave a field empty for no limit.",
    ) {
        current?.lines?.forEach { line ->
            RText(line, RType.sans(14f), c.secondary, Modifier.padding(horizontal = 16.dp, vertical = 4.dp))
        }
        Column(Modifier.padding(16.dp)) {
            Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                Column(Modifier.weight(1f)) {
                    FieldLabel("Most a purchase ($currency)")
                    RTextField(perPurchase, { perPurchase = it }, "No limit", tag = "budgetPerPurchase", keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Decimal))
                }
                Column(Modifier.weight(1f)) {
                    FieldLabel("Most in 30 days ($currency)")
                    RTextField(perMonth, { perMonth = it }, "No limit", tag = "budgetPerMonth", keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Decimal))
                }
            }
            Spacer(Modifier.height(10.dp))
            FieldLabel("Only these stores")
            RTextField(stores, { stores = it }, "Any store", tag = "budgetStores")
            Spacer(Modifier.height(12.dp))
            val purchase = perPurchase.takeIf { it.isNotBlank() }?.let { Money.parse(it, currency) }
            val month = perMonth.takeIf { it.isNotBlank() }?.let { Money.parse(it, currency) }
            val valid = (perPurchase.isBlank() || purchase != null) && (perMonth.isBlank() || month != null)
            CapsuleButton("Save budget", Modifier.fillMaxWidth().testTag("budgetSave"), style = ButtonStyle.Secondary, enabled = valid && !busy) {
                viewModel.setBudget(
                    BudgetView(
                        connectionId = "",
                        connectionLabel = "",
                        currency = currency,
                        perPurchase = purchase,
                        perDay = current?.perDay,
                        perMonth = month,
                        merchants = stores.split(',', ' ').map { it.trim() }.filter { it.isNotEmpty() },
                        lines = emptyList(),
                    ),
                )
            }
        }
    }
}
