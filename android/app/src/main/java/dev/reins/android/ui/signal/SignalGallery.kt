package dev.reins.android.ui.signal

import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ColumnScope
import androidx.compose.foundation.layout.ExperimentalLayoutApi
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.navigationBars
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.windowInsetsPadding
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.alpha
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.unit.dp
import dev.reins.android.design.Appearance
import dev.reins.android.design.AppearanceChoice
import dev.reins.android.design.AskText
import dev.reins.android.design.AutopilotPick
import dev.reins.android.design.ButtonSize
import dev.reins.android.design.ButtonStyle
import dev.reins.android.design.Calm
import dev.reins.android.design.ChipMode
import dev.reins.android.design.Choice
import dev.reins.android.design.Chooser
import dev.reins.android.design.ChooserSummary
import dev.reins.android.design.DockedTabBar
import dev.reins.android.design.Field
import dev.reins.android.design.FieldAvatar
import dev.reins.android.design.FieldHeader
import dev.reins.android.design.Glyph
import dev.reins.android.design.GlyphIcon
import dev.reins.android.design.HairlineRow
import dev.reins.android.design.HoldButton
import dev.reins.android.design.Illustration
import dev.reins.android.design.KeyValueRow
import dev.reins.android.design.KindTile
import dev.reins.android.design.LabelText
import dev.reins.android.design.LocalColors
import dev.reins.android.design.LocalSignalPrefs
import dev.reins.android.design.MenuItem
import dev.reins.android.design.MenuTint
import dev.reins.android.design.ModeChip
import dev.reins.android.design.Pick
import dev.reins.android.design.Pill
import dev.reins.android.design.Providers
import dev.reins.android.design.RText
import dev.reins.android.design.RType
import dev.reins.android.design.RollingNumber
import dev.reins.android.design.RoundIconButton
import dev.reins.android.design.SignalButton
import dev.reins.android.design.SignalCheckbox
import dev.reins.android.design.SignalChip
import dev.reins.android.design.SignalDialogPanel
import dev.reins.android.design.SignalMenu
import dev.reins.android.design.SignalMenuPanel
import dev.reins.android.design.SignalPrefs
import dev.reins.android.design.SignalRadio
import dev.reins.android.design.StatusWord
import dev.reins.android.design.Stepper
import dev.reins.android.design.TabItem
import dev.reins.android.design.TimerPill
import dev.reins.android.design.Toggle
import dev.reins.android.design.fieldSurface
import dev.reins.android.design.hairlineBelow

/** The pages of the gallery, each one screenshot. */
enum class GalleryPage(val title: String) {
    Type("Type"),
    Colour("Colour"),
    Controls("Controls"),
    Cards("Cards"),
    Glyphs("Glyphs"),
}

/**
 * Debug builds only (Settings, Developer): every Signal component, field (with its grain), type style and glyph, in
 * the current theme. The phase-1 screenshots come from here.
 */
@Composable
fun SignalGallery(onBack: () -> Unit, initialPage: GalleryPage = GalleryPage.Type) {
    val c = LocalColors.current
    var page by rememberSaveable { mutableStateOf(initialPage) }
    Column(Modifier.fillMaxSize().background(c.paper).testTag("signalGallery")) {
        FieldHeader(Field.Grant) {
            Row(verticalAlignment = Alignment.CenterVertically) {
                RoundIconButton(Glyph.ArrowBack, "Back", onField = true, onClick = onBack)
                Spacer(Modifier.width(12.dp))
                LabelText("Reins · Signal · ${page.title}", c.onField2, Modifier.weight(1f))
                TimerPill(72, 120)
            }
            AskText("The Signal gallery, every part", c.onField, target = "every part")
        }
        Row(
            Modifier.fillMaxWidth().horizontalScroll(rememberScrollState()).padding(horizontal = 24.dp, vertical = 14.dp),
            horizontalArrangement = Arrangement.spacedBy(6.dp),
        ) {
            GalleryPage.entries.forEach { p ->
                SignalChip(p.title, p == page, Modifier.testTag("galleryPage:${p.name}")) { page = p }
            }
        }
        Column(
            Modifier
                .weight(1f)
                .fillMaxWidth()
                .verticalScroll(rememberScrollState())
                .windowInsetsPadding(WindowInsets.navigationBars)
                .padding(horizontal = 24.dp)
                .padding(bottom = 32.dp),
            verticalArrangement = Arrangement.spacedBy(26.dp),
        ) {
            when (page) {
                GalleryPage.Type -> TypePage()
                GalleryPage.Colour -> ColourPage()
                GalleryPage.Controls -> ControlsPage()
                GalleryPage.Cards -> CardsPage()
                GalleryPage.Glyphs -> GlyphsPage()
            }
        }
    }
}

@Composable
private fun Section(title: String, content: @Composable ColumnScope.() -> Unit) {
    val c = LocalColors.current
    Column(verticalArrangement = Arrangement.spacedBy(14.dp)) {
        LabelText(title, c.inkMeta)
        content()
    }
}

// ---- Type ---------------------------------------------------------------------------------------------------------

@Composable
private fun TypePage() {
    val c = LocalColors.current
    val context = LocalContext.current
    val appearance by Appearance.state(context)
    Section("Serif") {
        TypeRow("ask · 45/46") { AskText("Push 8 commits to octo/app", c.ink, target = "octo/app") }
        TypeRow("title · 44") { RText("Activity", RType.title, c.ink) }
        TypeRow("stat · 40") { RText("142", RType.stat, c.ink) }
        TypeRow("sheet-title · 32/34") { androidx.compose.foundation.text.BasicText(dev.reins.android.design.askString("Remember for", "for"), style = RType.sheetTitle.copy(color = c.ink)) }
        TypeRow("ask-small · 25/26") { androidx.compose.foundation.text.BasicText(dev.reins.android.design.askString("Read 2 emails from Stripe", "Stripe"), style = RType.askSmall.copy(color = c.ink)) }
        TypeRow("mood · italic 24/30") { RText("All quiet. Three AIs are connected.", RType.mood, c.ink2) }
    }
    Section("Sans · Sora") {
        TypeRow("button · 16.5/600") { RText("Approve", RType.button, c.ink) }
        TypeRow("row · 15.5/500") { RText("Push to feature/* in octo/app", RType.row, c.ink) }
        TypeRow("body · 15/22") { RText("Approved without asking. Sent means sent.", RType.body, c.ink2) }
        TypeRow("meta · 13/18") { RText("Claude Code · 3 of 10 uses", RType.meta, c.inkMeta) }
        TypeRow("tag · 11.5/600") { Pill("Auto", c.violet) }
    }
    Section("Mono") {
        TypeRow("label · 11.5 +0.16em") { LabelText("GitHub · Push", c.inkMeta) }
        TypeRow("value · 13.5") { RText("feature/passkeys · 9f86d08", RType.value, c.ink) }
    }
    Section("Ask at the smallest it shrinks to (32)") {
        AskText("Delete the repository octo/old-api and every one of its forks, issues and releases", c.ink, target = "octo/old-api")
    }
    // The sans and the grain are chosen in Settings › Appearance.
}

@Composable
private fun TypeRow(name: String, sample: @Composable () -> Unit) {
    val c = LocalColors.current
    Column(Modifier.fillMaxWidth(), verticalArrangement = Arrangement.spacedBy(4.dp)) {
        LabelText(name, c.inkFaint)
        sample()
    }
}

// ---- Colour -------------------------------------------------------------------------------------------------------

private val fieldWords = mapOf(
    Field.Push to ("Send" to "it"),
    Field.Read to ("Look" to "inside"),
    Field.Delete to ("For" to "good"),
    Field.Grant to ("For a" to "while"),
    Field.Pair to ("Say" to "hello"),
    Field.Accounts to ("Keys" to "change"),
    Field.Warn to ("Fix" to "me"),
)

@OptIn(ExperimentalLayoutApi::class)
@Composable
private fun ColourPage() {
    val c = LocalColors.current
    Section("Fields · the same in both themes") {
        FlowRow(horizontalArrangement = Arrangement.spacedBy(10.dp), verticalArrangement = Arrangement.spacedBy(10.dp), maxItemsInEachRow = 3) {
            Field.entries.forEach { f ->
                val (a, b) = fieldWords.getValue(f)
                Column(
                    Modifier.weight(1f).height(108.dp).fieldSurface(f, RoundedCornerShape(18.dp)).padding(12.dp).testTag("field:${f.name}"),
                    verticalArrangement = Arrangement.SpaceBetween,
                ) {
                    LabelText(f.name, c.onField2)
                    androidx.compose.foundation.text.BasicText(dev.reins.android.design.askString("$a $b", b), style = RType.serif(22f, 22f).copy(color = c.onField))
                }
            }
            Spacer(Modifier.weight(1f))
            Spacer(Modifier.weight(1f))
        }
    }
    Section("Flat · before    pigment · after") {
        Row(horizontalArrangement = Arrangement.spacedBy(10.dp)) {
            CompositionLocalProvider(LocalSignalPrefs provides SignalPrefs(grain = false)) {
                Box(Modifier.weight(1f).height(96.dp).fieldSurface(Field.Push, RoundedCornerShape(18.dp)).padding(12.dp).testTag("flat")) {
                    LabelText("Flat", c.onField2)
                }
            }
            CompositionLocalProvider(LocalSignalPrefs provides SignalPrefs(grain = true)) {
                Box(Modifier.weight(1f).height(96.dp).fieldSurface(Field.Push, RoundedCornerShape(18.dp)).padding(12.dp).testTag("pigment")) {
                    LabelText("Pigment", c.onField2)
                }
            }
        }
    }
    Section("Paper and ink") {
        Column {
            listOf(
                Triple("paper", c.paper, "ink"), Triple("paper-raised", c.paperRaised, "ink-2"), Triple("paper-sunken", c.paperSunken, "ink-meta"),
                Triple("rule", c.rule, "ink-faint"), Triple("violet", c.violet, "success"), Triple("danger", c.danger, "warning"),
            ).zip(listOf(c.ink, c.ink2, c.inkMeta, c.inkFaint, c.success, c.warning)).forEach { (left, right) ->
                Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                    Swatch(left.first, left.second, Modifier.weight(1f))
                    Swatch(left.third, right, Modifier.weight(1f))
                }
            }
        }
    }
    Section("Kind tiles · the ink at 13 %") {
        Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            listOf(
                Field.Push to Glyph.Send, Field.Read to Glyph.Search, Field.Delete to Glyph.Trash, Field.Grant to Glyph.Key,
                Field.Pair to Glyph.Link, Field.Accounts to Glyph.Person, Field.Warn to Glyph.Warning,
            ).forEach { (f, g) -> KindTile(f, g) }
        }
    }
}

@Composable
private fun Swatch(name: String, color: Color, modifier: Modifier) {
    val c = LocalColors.current
    Row(modifier.height(52.dp).hairlineBelow(c.rule), verticalAlignment = Alignment.CenterVertically) {
        Box(Modifier.size(30.dp).background(color, RoundedCornerShape(9.dp)).border(1.dp, c.rule, RoundedCornerShape(9.dp)))
        Spacer(Modifier.width(12.dp))
        RText(name, RType.sans(14.5f, androidx.compose.ui.text.font.FontWeight.Medium), c.ink, maxLines = 1)
    }
}

// ---- Controls -----------------------------------------------------------------------------------------------------

@OptIn(ExperimentalLayoutApi::class)
@Composable
private fun ControlsPage() {
    val c = LocalColors.current
    Section("Buttons") {
        Row(horizontalArrangement = Arrangement.spacedBy(10.dp)) {
            SignalButton("Deny", Modifier.weight(1f), style = ButtonStyle.Outline) {}
            SignalButton("Approve", Modifier.weight(1.35f), glyph = Glyph.Fingerprint) {}
        }
        FlowRow(horizontalArrangement = Arrangement.spacedBy(10.dp), verticalArrangement = Arrangement.spacedBy(10.dp)) {
            SignalButton("This was wrong", style = ButtonStyle.Soft, size = ButtonSize.Small) {}
            SignalButton("End now", style = ButtonStyle.DangerWash, size = ButtonSize.Small) {}
            SignalButton("Allow", style = ButtonStyle.Violet, size = ButtonSize.Small) {}
            SignalButton("Pressed", Modifier.alpha(0.55f), size = ButtonSize.Small) {}
            SignalButton("Disabled", size = ButtonSize.Small, enabled = false) {}
        }
        Row(
            Modifier.fillMaxWidth().fieldSurface(Field.Push, RoundedCornerShape(20.dp)).padding(14.dp),
            horizontalArrangement = Arrangement.spacedBy(8.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            SignalButton("Review", Modifier.weight(1f), style = ButtonStyle.OnFieldWhite, size = ButtonSize.Small) {}
            SignalButton("Deny", Modifier.weight(1f), style = ButtonStyle.OnFieldWash, size = ButtonSize.Small) {}
            RoundIconButton(Glyph.Close, "Close", onField = true) {}
        }
        Row(horizontalArrangement = Arrangement.spacedBy(12.dp), verticalAlignment = Alignment.CenterVertically) {
            SignalButton("Focused", size = ButtonSize.Small, showFocus = true) {}
            RoundIconButton(Glyph.Plus, "Add") {}
            RoundIconButton(Glyph.Filter, "Filter") {}
            RoundIconButton(Glyph.ChevronLeft, "Back") {}
        }
    }
    Section("Hold · irreversible") {
        HoldButton("Hold to delete", c.danger, Modifier.fillMaxWidth(), icon = Glyph.Trash.vector) {}
        HoldButton("Hold to delete", c.danger, Modifier.fillMaxWidth(), icon = Glyph.Trash.vector, previewProgress = 0.58f) {}
        HoldButton("Hold to delete", c.danger, Modifier.fillMaxWidth(), done = true, doneTitle = "Deleted") {}
        RText("1200 ms. Then the fingerprint. Letting go cancels and drains.", RType.meta, c.inkMeta)
    }
    Section("Allow for · the chooser") {
        var pick by rememberSaveable { mutableStateOf("once") }
        val more = listOf(Choice("8h", "8 hours"), Choice("week", "1 week"), Choice("month", "30 days"))
        Chooser(listOf(Choice("once", "Once"), Choice("hour", "1 hour"), Choice("day", "1 day")), pick, { pick = it }, more = more)
        Chooser(listOf(Choice("once", "Once"), Choice("8h", "8 hours"), Choice("day", "1 day")), "week", {}, more = more)
        Chooser(emptyList<Choice<String>>(), null, {}, locked = true)
        ChooserSummary("Allow for", "1 hour · at most 5 uses") {}
    }
    Section("Menus · grow from their trigger") {
        var open by rememberSaveable { mutableStateOf(false) }
        Box(Modifier.fillMaxWidth(), contentAlignment = Alignment.CenterEnd) {
            ModeChip(ChipMode.Auto, chevron = true) { open = true }
            SignalMenu(open, { open = false }, modeItems())
        }
        Box(Modifier.fillMaxWidth(), contentAlignment = Alignment.CenterEnd) { SignalMenuPanel(modeItems()) }
    }
    Section("Controls") {
        var on by rememberSaveable { mutableStateOf(true) }
        var n by rememberSaveable { mutableIntStateOf(5) }
        Row(horizontalArrangement = Arrangement.spacedBy(16.dp), verticalAlignment = Alignment.CenterVertically) {
            Toggle(on) { on = it }
            Toggle(false) {}
            SignalCheckbox(true)
            SignalCheckbox(false)
            SignalRadio(true)
            SignalRadio(false)
        }
        Row(horizontalArrangement = Arrangement.spacedBy(16.dp), verticalAlignment = Alignment.CenterVertically) {
            Stepper(n, { n = it })
            Toggle(true, enabled = false) {}
        }
        var filter by rememberSaveable { mutableStateOf("Autopilot") }
        Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            listOf("All", "Autopilot", "Denied", "Grants").forEach { SignalChip(it, it == filter) { filter = it } }
        }
    }
}

private fun modeItems() = listOf(
    MenuItem("Manual", "You decide, with a hint", Glyph.Person) {},
    MenuItem("Auto", "Decides the easy ones", Glyph.Autopilot, MenuTint.Violet, selected = true) {},
    MenuItem("Lockdown", "Deny everything now", Glyph.Lock, MenuTint.Warn) {},
    MenuItem("Bypass…", "Approve everything for a while", Glyph.Autopilot, MenuTint.Danger, destructive = true, chevron = true, divider = true) {},
)

// ---- Cards --------------------------------------------------------------------------------------------------------

@OptIn(ExperimentalLayoutApi::class)
@Composable
private fun CardsPage() {
    val c = LocalColors.current
    val claude = Providers.byKey("claude")
    val openai = Providers.byKey("openai")
    Section("Waiting cards · Autopilot's pick first") {
        dev.reins.android.design.WaitingCard(
            Field.Push, "Gmail · Send", "Send an email to Anna Petrova", target = "Anna Petrova",
            avatar = { claude?.let { FieldAvatar(it) } },
            timer = { TimerPill(72, 120) },
        ) {
            AutopilotPick(Pick.Sure(allow = true, percent = 97), Modifier.weight(1f)) {}
            SignalButton("Review", Modifier.weight(1f), style = ButtonStyle.OnFieldWash, size = ButtonSize.Small) {}
        }
        dev.reins.android.design.WaitingCard(
            Field.Read, "Gmail · Search", "Search mail for “invoice”", target = "“invoice”",
            avatar = { openai?.let { FieldAvatar(it) } },
            timer = { TimerPill(12, 120) },
        ) {
            AutopilotPick(Pick.Unsure) {}
            SignalButton("Review", Modifier.weight(1f), style = ButtonStyle.OnFieldWhite, size = ButtonSize.Small) {}
        }
        dev.reins.android.design.WaitingCard(
            Field.Delete, "GitHub · Delete", "Delete the repo octo/old-api", target = "octo/old-api",
            avatar = { claude?.let { FieldAvatar(it) } },
            timer = { RText("4:51", RType.mono(13f), c.onField) },
        ) {
            AutopilotPick(Pick.AlwaysAsks) {}
            SignalButton("Review", Modifier.weight(1f), style = ButtonStyle.OnFieldWhite, size = ButtonSize.Small) {}
        }
    }
    Section("Mode chip · in every header") {
        FlowRow(horizontalArrangement = Arrangement.spacedBy(8.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
            ModeChip(ChipMode.Manual)
            ModeChip(ChipMode.Auto)
            ModeChip(ChipMode.Lockdown)
            ModeChip(ChipMode.Bypass, timer = "14:05")
            ModeChip(ChipMode.Overnight)
        }
    }
    Section("Rows · hairlines, never cards") {
        Column {
            KeyValueRow("Branch") {
                RText("feature/passkeys", RType.value, c.ink, maxLines = 1, ltr = true)
                Pill("New", c.success)
            }
            KeyValueRow("8 commits", chevron = true, onClick = {}) {
                RText("+412", RType.mono(13f), c.success)
                RText("−96", RType.mono(13f), c.danger)
            }
            HairlineRow("Pushed 8 commits to octo/app", subtitle = "Claude Code · 4 min", leading = { KindTile(Field.Push, Glyph.GitBranch) }, trailing = { StatusWord("Auto", c.violet) }, onClick = {})
            HairlineRow("Read 2 emails from Stripe", subtitle = "ChatGPT · 18 min", leading = { KindTile(Field.Read, Glyph.Mail) }, trailing = { GlyphIcon(Glyph.Check, c.success, size = 18.dp) }, onClick = {})
            HairlineRow("Delete branch old/experiment", subtitle = "Cursor · 1 h", leading = { KindTile(Field.Delete, Glyph.Trash) }, trailing = { StatusWord("Denied", c.danger) }, onClick = {})
            var wifi by rememberSaveable { mutableStateOf(true) }
            HairlineRow("Download on Wi-Fi only", subtitle = "The model is 370 MB", divider = false, trailing = { Toggle(wifi) { wifi = it } })
        }
    }
    Section("Numbers · timer") {
        var count by rememberSaveable { mutableIntStateOf(142) }
        Row(horizontalArrangement = Arrangement.spacedBy(12.dp), verticalAlignment = Alignment.CenterVertically) {
            RollingNumber(count.toString(), RType.stat, c.ink)
            RText("requests", RType.serif(22f), c.ink2)
            Spacer(Modifier.weight(1f))
            SignalButton("+1", style = ButtonStyle.Soft, size = ButtonSize.Small) { count++ }
        }
        Row(
            Modifier.fillMaxWidth().fieldSurface(Field.Read, RoundedCornerShape(18.dp)).padding(14.dp),
            horizontalArrangement = Arrangement.spacedBy(14.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            TimerPill(72, 120)
            TimerPill(40, 120)
            TimerPill(12, 120)
        }
    }
    Section("Tab bar · Android docks it") {
        var tab by rememberSaveable { mutableIntStateOf(0) }
        Box(Modifier.fillMaxWidth().border(1.dp, c.rule, RoundedCornerShape(18.dp)).padding(1.dp)) {
            DockedTabBar(
                listOf(
                    TabItem("Activity", Glyph.Activity, badge = 1),
                    TabItem("Trust", Glyph.Shield),
                    TabItem("Connections", Glyph.Link),
                    TabItem("Settings", Glyph.Settings),
                ),
                tab,
                { tab = it },
            )
        }
    }
    Section("Dialog") {
        SignalDialogPanel("Bypass every AI?", "Everything is approved without asking until you end it.", "Bypass", {}, {}, destructive = true, italic = "every AI?")
    }
    Section("Calm · ink drawings for empty states") {
        Calm("All quiet on the water.")
        Calm("Off it goes.", illustration = Illustration.Kite)
    }
}

// ---- Glyphs -------------------------------------------------------------------------------------------------------

@OptIn(ExperimentalLayoutApi::class)
@Composable
private fun GlyphsPage() {
    val c = LocalColors.current
    Section("${Glyph.entries.size} glyphs · 24 grid · 1.6 stroke") {
        FlowRow(horizontalArrangement = Arrangement.spacedBy(4.dp), verticalArrangement = Arrangement.spacedBy(10.dp), maxItemsInEachRow = 6) {
            Glyph.entries.forEach { g ->
                Column(Modifier.weight(1f), horizontalAlignment = Alignment.CenterHorizontally, verticalArrangement = Arrangement.spacedBy(4.dp)) {
                    GlyphIcon(g, c.ink, size = 22.dp)
                    RText(g.name, RType.sans(8.5f), c.inkMeta, maxLines = 1)
                }
            }
        }
    }
}
