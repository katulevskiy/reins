package dev.reins.android.ui.approval

import androidx.compose.animation.AnimatedVisibility
import androidx.compose.animation.core.animateFloatAsState
import androidx.compose.animation.core.tween
import androidx.compose.animation.expandVertically
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.animation.shrinkVertically
import androidx.compose.animation.slideInVertically
import androidx.compose.animation.slideOutVertically
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.gestures.detectTapGestures
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ExperimentalLayoutApi
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.defaultMinSize
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.navigationBars
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.windowInsetsPadding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.selection.selectable
import androidx.compose.foundation.selection.toggleable
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.rounded.Add
import androidx.compose.material.icons.rounded.AllInclusive
import androidx.compose.material.icons.rounded.Check
import androidx.compose.material.icons.rounded.ChevronRight
import androidx.compose.material.icons.rounded.Close
import androidx.compose.material.icons.rounded.Info
import androidx.compose.material.icons.rounded.KeyboardArrowDown
import androidx.compose.material.icons.rounded.Remove
import androidx.compose.material.icons.rounded.Warning
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.Icon
import androidx.compose.material3.Slider
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.draw.rotate
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import dev.reins.android.autopilot.AutopilotText
import dev.reins.android.design.HoldButton
import dev.reins.android.design.LocalColors
import dev.reins.android.design.RText
import dev.reins.android.design.RTextField
import dev.reins.android.design.RType
import dev.reins.android.design.Toggle
import dev.reins.android.design.pressable
import dev.reins.android.design.serviceName
import dev.reins.android.feedback.Event
import dev.reins.android.feedback.LocalFeedback
import dev.reins.android.feedback.detentAction
import dev.reins.android.feedback.play
import dev.reins.android.ui.autopilot.MeterBar
import dev.reins.android.ui.autopilot.NeighbourRow
import dev.reins.android.ui.autopilot.verdictTint
import dev.reins.android.ui.common.untrusted
import dev.reins.core.ApprovalKind
import dev.reins.core.ApprovalView
import dev.reins.core.ResourceView
import dev.reins.core.SuggestionView
import dev.reins.core.Verdict

/** The sheets of the approval screen, over a scrim: "Remember for", Autopilot's reasons, a permission's length. */
@Composable
internal fun ApprovalOverlays(view: ApprovalView, ui: ApprovalUi, viewModel: ApprovalViewModel, onOpenAutopilot: () -> Unit) {
    val c = LocalColors.current
    val nav = LocalApprovalNav.current
    val feedback = LocalFeedback.current
    val shown = nav.overlay
    var last by remember { mutableStateOf(shown) }
    if (shown != null) last = shown
    val dismiss = {
        feedback.play(Event.Close)
        nav.overlay = null
    }
    BoxWithConstraints(Modifier.fillMaxSize()) {
        val tallest = maxHeight * 0.92f
        AnimatedVisibility(shown != null, enter = fadeIn(tween(180)), exit = fadeOut(tween(160))) {
            Box(
                Modifier
                    .fillMaxSize()
                    .background(Color.Black.copy(alpha = 0.6f))
                    .clickable(interactionSource = remember { MutableInteractionSource() }, indication = null, onClick = dismiss),
            )
        }
        AnimatedVisibility(
            shown != null,
            modifier = Modifier.align(Alignment.BottomCenter),
            enter = slideInVertically(tween(260)) { it },
            exit = slideOutVertically(tween(200)) { it },
        ) {
            val shape = RoundedCornerShape(topStart = 28.dp, topEnd = 28.dp)
            Column(
                Modifier
                    .fillMaxWidth()
                    .heightIn(max = tallest)
                    .clip(shape)
                    .background(Surfaces.sheet(c))
                    .border(1.dp, if (c.dark) Color(0xFF26262B) else c.hairline, shape)
                    // Taps on the sheet stay on it (no semantics, so the content keeps its own).
                    .pointerInput(Unit) { detectTapGestures {} }
                    .windowInsetsPadding(WindowInsets.navigationBars)
                    .padding(bottom = 16.dp),
            ) {
                Box(Modifier.fillMaxWidth().padding(top = 6.dp, bottom = 14.dp), contentAlignment = Alignment.Center) {
                    Box(Modifier.width(32.dp).height(4.dp).clip(CircleShape).background(if (c.dark) Color(0xFF3A3A40) else c.hairline))
                }
                when (last) {
                    ApprovalOverlay.Remember -> RememberSheet(view, ui, viewModel, dismiss)
                    ApprovalOverlay.Autopilot -> ui.suggestion?.let { AutopilotSheet(it, dismiss) { nav.overlay = null; onOpenAutopilot() } }
                    ApprovalOverlay.Grant -> GrantSheet(view, ui, viewModel, dismiss)
                    null -> Unit
                }
            }
        }
    }
}

@Composable
private fun SheetHead(title: String, onClose: () -> Unit) {
    val c = LocalColors.current
    Row(Modifier.fillMaxWidth().padding(start = 22.dp, end = 12.dp, bottom = 12.dp), verticalAlignment = Alignment.CenterVertically) {
        RText(title, RType.sans(18f, FontWeight.SemiBold), c.text, Modifier.weight(1f), maxLines = 1)
        Box(
            Modifier.size(44.dp).pressable(shape = CircleShape, label = "Close", onClick = onClose).semantics { contentDescription = "Close" }.testTag("closeOverlay"),
            contentAlignment = Alignment.Center,
        ) { Icon(Icons.Rounded.Close, null, Modifier.size(24.dp), tint = c.text) }
    }
}

// ---- Remember for ---------------------------------------------------------------------------------------------------

/**
 * "Remember for": a week, 30 days, a custom time, a number of uses, and, set apart in red, until revoked. Under More,
 * what a remembered approval covers. The choice shows in the dock's last segment.
 */
@Composable
private fun RememberSheet(view: ApprovalView, ui: ApprovalUi, viewModel: ApprovalViewModel, onClose: () -> Unit) {
    val c = LocalColors.current
    val feedback = LocalFeedback.current
    val draft = ui.draft
    var pending by remember { mutableStateOf(draft.lifetime.takeIf { it != LifetimeKind.ONCE }) }
    var more by remember { mutableStateOf(false) }
    val forever = pending == LifetimeKind.UNTIL_REVOKED
    val confirm = {
        val chosen = pending
        if (chosen != null) viewModel.edit { it.copy(lifetime = chosen, allMail = null) }
        onClose()
    }
    Column(Modifier.testTag("rememberSheet")) {
        SheetHead("Remember for", onClose)
        Column(Modifier.weight(1f, fill = false).verticalScroll(rememberScrollState())) {
            FactGroup(
                rows = sheetLifetimes(view).map { kind ->
                    { LifetimeOption(kind, pending == kind && draft.allMail == null, draft, viewModel) { pending = kind } }
                },
            )
            if (offersUntilRevoked(view)) {
                Spacer(Modifier.height(12.dp))
                FactGroup(
                    borderTint = c.danger.copy(alpha = 0.3f),
                    rows = listOf {
                        OptionRow(forever && draft.allMail == null, "lifetime:UNTIL_REVOKED", danger = true, onSelect = { pending = LifetimeKind.UNTIL_REVOKED }) {
                            Icon(Icons.Rounded.AllInclusive, null, Modifier.size(20.dp), tint = c.danger)
                            Spacer(Modifier.width(12.dp))
                            Column(Modifier.weight(1f)) {
                                RText("Until I revoke", RType.sans(15.5f, FontWeight.SemiBold), c.danger, maxLines = 1)
                                RText("Never expires", RType.sans(12.5f), c.tertiary, Modifier.padding(top = 2.dp), maxLines = 1)
                            }
                        }
                    },
                )
            }
            AnimatedVisibility(forever, enter = expandVertically() + fadeIn(), exit = shrinkVertically() + fadeOut()) {
                Row(
                    Modifier
                        .padding(start = 16.dp, end = 16.dp, top = 10.dp)
                        .fillMaxWidth()
                        .clip(RoundedCornerShape(16.dp))
                        .background(c.danger.copy(alpha = 0.10f))
                        .border(1.dp, c.danger.copy(alpha = 0.28f), RoundedCornerShape(16.dp))
                        .padding(horizontal = 14.dp, vertical = 12.dp)
                        .testTag("foreverWarning"),
                ) {
                    Icon(Icons.Rounded.Warning, null, Modifier.size(18.dp).padding(top = 1.dp), tint = c.danger)
                    Spacer(Modifier.width(10.dp))
                    Column {
                        RText("Anything like this runs without asking, forever.", RType.sans(13.5f, FontWeight.SemiBold, lineHeight = 19f), c.danger)
                        RText("Only for things you'd never want to stop.", RType.sans(13.5f, lineHeight = 19f), if (c.dark) Color(0xFFFFBBBB) else c.danger)
                    }
                }
            }
            if (view.kind != ApprovalKind.ACCOUNTS) {
                Spacer(Modifier.height(12.dp))
                MoreToggle(more) {
                    more = !more
                    feedback.play(Event.expand(more))
                }
                AnimatedVisibility(more, enter = expandVertically() + fadeIn(), exit = shrinkVertically() + fadeOut()) {
                    Column(Modifier.padding(top = 4.dp)) {
                        if (view.envRelease != null) EnvScopeOptions(view, ui, viewModel) else ScopeOptions(view, ui, viewModel)
                    }
                }
            }
        }
        Box(Modifier.padding(start = 16.dp, end = 16.dp, top = 16.dp)) {
            if (forever && draft.allMail == null) {
                Column(horizontalAlignment = Alignment.CenterHorizontally) {
                    HoldButton("Remember until I revoke", c.danger, Modifier.fillMaxWidth().testTag("rememberConfirm"), icon = null, onConfirm = confirm)
                    RText("Hold for a second", RType.sans(12f), c.tertiary, Modifier.padding(top = 8.dp))
                }
            } else {
                val label = when {
                    draft.allMail != null -> "Done"
                    pending == null -> "Choose how long"
                    else -> rememberButton(view, draft.copy(lifetime = pending!!))
                }
                val enabled = draft.allMail != null || pending != null
                Row(
                    Modifier
                        .fillMaxWidth()
                        .height(54.dp)
                        .clip(CircleShape)
                        .background(if (enabled) c.text else Surfaces.raised(c))
                        .pressable(enabled = enabled, shape = CircleShape, onClick = confirm)
                        .testTag("rememberConfirm"),
                    horizontalArrangement = Arrangement.Center,
                    verticalAlignment = Alignment.CenterVertically,
                ) { RText(label, RType.sans(16.5f, FontWeight.SemiBold), if (enabled) c.background else c.tertiary, maxLines = 1) }
            }
        }
    }
}

/** A radio row. */
@Composable
private fun OptionRow(
    selected: Boolean,
    tag: String,
    danger: Boolean = false,
    onSelect: () -> Unit,
    content: @Composable androidx.compose.foundation.layout.RowScope.() -> Unit,
) {
    val c = LocalColors.current
    val feedback = LocalFeedback.current
    Row(
        Modifier
            .fillMaxWidth()
            .defaultMinSize(minHeight = 52.dp)
            .selectable(selected, role = Role.RadioButton) {
                onSelect()
                feedback.play(Event.Selection)
            }
            .padding(start = 18.dp, end = 16.dp, top = 10.dp, bottom = 10.dp)
            .testTag(tag),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        content()
        Spacer(Modifier.width(10.dp))
        Radio(selected, if (danger) c.danger else c.accent)
    }
}

@Composable
private fun Radio(on: Boolean, tint: Color) {
    val c = LocalColors.current
    Box(
        Modifier.size(22.dp).clip(CircleShape).border(2.dp, if (on) tint else (if (c.dark) Color(0xFF4A4A52) else c.tertiary), CircleShape),
        contentAlignment = Alignment.Center,
    ) { if (on) Box(Modifier.size(11.dp).clip(CircleShape).background(tint)) }
}

@Composable
private fun LifetimeOption(kind: LifetimeKind, selected: Boolean, draft: ApprovalDraft, viewModel: ApprovalViewModel, onSelect: () -> Unit) {
    val c = LocalColors.current
    OptionRow(selected, "lifetime:${kind.name}", onSelect = onSelect) {
        val title = when (kind) {
            LifetimeKind.WEEK -> "1 week"
            LifetimeKind.MONTH -> "30 days"
            LifetimeKind.CUSTOM -> "Custom"
            LifetimeKind.USES -> "Number of uses"
            else -> kind.label
        }
        RText(title, RType.sans(15.5f), c.text, Modifier.weight(1f), maxLines = 1)
        when (kind) {
            LifetimeKind.CUSTOM -> {
                Stepper(draft.customAmount, 1, draft.customUnit.max, "custom") { n -> viewModel.edit { it.copy(customAmount = n) }; onSelect() }
                Spacer(Modifier.width(6.dp))
                UnitMenu(draft.customUnit) { u -> viewModel.edit { it.copy(customUnit = u, customAmount = it.customAmount.coerceAtMost(u.max)) }; onSelect() }
            }
            LifetimeKind.USES -> Stepper(draft.uses, 1, MAX_USES, "uses") { n -> viewModel.edit { it.copy(uses = n) }; onSelect() }
            else -> Unit
        }
    }
}

/** − n + on a sunken plate. */
@Composable
private fun Stepper(value: Int, min: Int, max: Int, tag: String, onChange: (Int) -> Unit) {
    val c = LocalColors.current
    val feedback = LocalFeedback.current
    Row(Modifier.clip(RoundedCornerShape(11.dp)).background(Surfaces.sunken(c)), verticalAlignment = Alignment.CenterVertically) {
        StepButton(Icons.Rounded.Remove, "Fewer", value > min, "$tag:minus") {
            onChange(value - 1)
            feedback.play(Event.Detent, value)
        }
        RText(value.toString(), RType.mono(15f, FontWeight.Medium), c.text, Modifier.width(34.dp).testTag("$tag:value"), maxLines = 1, align = androidx.compose.ui.text.style.TextAlign.Center)
        StepButton(Icons.Rounded.Add, "More", value < max, "$tag:plus") {
            onChange(value + 1)
            feedback.play(Event.Detent, value + 2)
        }
    }
}

@Composable
private fun StepButton(icon: androidx.compose.ui.graphics.vector.ImageVector, label: String, enabled: Boolean, tag: String, onClick: () -> Unit) {
    val c = LocalColors.current
    Box(
        Modifier.size(width = 34.dp, height = 36.dp).pressable(enabled = enabled, shape = RoundedCornerShape(11.dp), label = label, onClick = onClick).testTag(tag),
        contentAlignment = Alignment.Center,
    ) { Icon(icon, label, Modifier.size(16.dp), tint = if (enabled) c.secondary else c.tertiary.copy(alpha = 0.5f)) }
}

@Composable
private fun UnitMenu(unit: LifetimeUnit, onPick: (LifetimeUnit) -> Unit) {
    val c = LocalColors.current
    var open by remember { mutableStateOf(false) }
    Box {
        Row(
            Modifier
                .clip(RoundedCornerShape(11.dp))
                .background(Surfaces.sunken(c))
                .pressable(shape = RoundedCornerShape(11.dp)) { open = true }
                .padding(start = 11.dp, end = 8.dp, top = 8.dp, bottom = 8.dp)
                .testTag("custom:unit"),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            RText(unit.many, RType.sans(14.5f, FontWeight.SemiBold), c.text, maxLines = 1)
            Spacer(Modifier.width(3.dp))
            Icon(Icons.Rounded.KeyboardArrowDown, null, Modifier.size(15.dp), tint = c.secondary)
        }
        DropdownMenu(open, onDismissRequest = { open = false }) {
            LifetimeUnit.entries.forEach { u ->
                DropdownMenuItem(
                    text = { RText(u.many, RType.sans(15f), c.text) },
                    onClick = {
                        open = false
                        onPick(u)
                    },
                    modifier = Modifier.testTag("unit:${u.name}"),
                )
            }
        }
    }
}

@Composable
private fun MoreToggle(open: Boolean, onToggle: () -> Unit) {
    val c = LocalColors.current
    val turn by animateFloatAsState(if (open) 90f else 0f, label = "more")
    Row(
        Modifier
            .padding(horizontal = 16.dp)
            .fillMaxWidth()
            .clip(RoundedCornerShape(16.dp))
            .pressable(highlight = c.controlFill, shape = RoundedCornerShape(16.dp), onClick = onToggle)
            .padding(horizontal = 6.dp, vertical = 12.dp)
            .testTag("moreToggle"),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        RText("What it covers", RType.sans(15f, FontWeight.Medium), c.accent, Modifier.weight(1f), maxLines = 1)
        Icon(Icons.Rounded.ChevronRight, null, Modifier.size(20.dp).rotate(turn), tint = c.accent)
    }
}

/** What a remembered approval covers: anything like this (the default), or exactly what is ticked here. */
@OptIn(ExperimentalLayoutApi::class)
@Composable
private fun ScopeOptions(view: ApprovalView, ui: ApprovalUi, viewModel: ApprovalViewModel) {
    val c = LocalColors.current
    val draft = ui.draft
    val like = view.quick?.allow
    val write = view.kind == ApprovalKind.WRITE
    val send = view.kind == ApprovalKind.SEND
    val gmail = view.kind == ApprovalKind.SEARCH || view.kind == ApprovalKind.READ || send
    val pick: (ApprovalDraft) -> ApprovalDraft = { it.copy(likeThis = false) }
    Column(verticalArrangement = Arrangement.spacedBy(10.dp)) {
        if (like != null) {
            FactGroup(
                rows = listOf(
                    {
                        OptionRow(!draft.likeThis && draft.allMail == null, "scope:picked", onSelect = { viewModel.edit { it.copy(likeThis = false, allMail = null) } }) {
                            RText("Only what's shown", RType.sans(15.5f), c.text, Modifier.weight(1f), maxLines = 1)
                        }
                    },
                    {
                        // Wider than the screen: it says exactly what, and the dock says it is on.
                        OptionRow(draft.likeThis && draft.allMail == null, "scope:like", danger = true, onSelect = { viewModel.edit { it.copy(likeThis = true, allMail = null) } }) {
                            Column(Modifier.weight(1f)) {
                                RText("Anything like this", RType.sans(15.5f, FontWeight.Medium), c.warning, maxLines = 1)
                                RText(
                                    "${untrusted(view.quick!!.allowWhat)}, by ${untrusted(view.connectionLabel)}",
                                    RType.sans(12.5f),
                                    c.tertiary,
                                    Modifier.padding(top = 2.dp).testTag("allowWhat"),
                                    maxLines = 2,
                                )
                            }
                        }
                    },
                ),
            )
        }
        if (!write && !send) EverythingChips(view, draft, viewModel)
        if (draft.allMail == null) {
            if (gmail) {
                GmailScope(view, draft, viewModel, pick)
            } else {
                if (write && view.classes.isNotEmpty()) {
                    GroupHeader("Kinds of change")
                    FlowRow(
                        Modifier.padding(horizontal = 16.dp).testTag("classes"),
                        horizontalArrangement = Arrangement.spacedBy(8.dp),
                        verticalArrangement = Arrangement.spacedBy(8.dp),
                    ) {
                        view.classes.forEach { kind ->
                            val on = kind.id in draft.classes
                            Chip(untrusted(kind.label), on, "class:${kind.id}") { viewModel.edit { pick(it).copy(classes = toggleClass(it.classes, kind.id, !on)) } }
                        }
                    }
                }
                val (wide, narrow) = view.resources.partition { it.wider }
                ResourceRows(narrow, draft, viewModel, pick)
                if (wide.isNotEmpty()) {
                    GroupHeader("Or wider", Modifier.testTag("widerCaption"))
                    ResourceRows(wide, draft, viewModel, pick)
                }
            }
        }
    }
}

/** A project's environment: which command pattern a remembered release covers (narrowest first), and every variable. */
@Composable
private fun EnvScopeOptions(view: ApprovalView, ui: ApprovalUi, viewModel: ApprovalViewModel) {
    val c = LocalColors.current
    val release = view.envRelease ?: return
    val picked = envCommandOf(view, ui.draft)?.id
    Column(verticalArrangement = Arrangement.spacedBy(10.dp)) {
        GroupHeader("Command")
        FactGroup(
            tag = "envCommands",
            rows = release.commands.map { option ->
                {
                    OptionRow(picked == option.id, "envCommand:${option.id}", onSelect = { viewModel.edit { it.copy(envCommand = option.id) } }) {
                        RText(untrusted(option.label), RType.mono(14f, FontWeight.Medium), c.text, Modifier.weight(1f), ltr = true)
                    }
                }
            },
        )
        if (!release.allVariables) {
            FactGroup(
                rows = listOf {
                    CheckOption(ui.draft.envAllVariables, "Every variable", "envAllVariables") { on -> viewModel.edit { it.copy(envAllVariables = on) } }
                },
            )
        }
    }
}

/** All mail (or all of an integration) for a while: never open-ended, never for changing things. */
@OptIn(ExperimentalLayoutApi::class)
@Composable
private fun EverythingChips(view: ApprovalView, draft: ApprovalDraft, viewModel: ApprovalViewModel) {
    val gmail = view.service.isBlank() || view.service == "gmail"
    GroupHeader(if (gmail) "All mail for" else "All of ${serviceName(view.service)} for")
    FlowRow(Modifier.padding(horizontal = 16.dp), horizontalArrangement = Arrangement.spacedBy(8.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
        EVERYTHING_LIFETIMES.forEach { kind ->
            Chip(periodWords(kind.seconds!!), draft.allMail == kind, "allMail:${kind.name}") {
                viewModel.edit {
                    if (it.allMail == kind) it.copy(allMail = null) else it.copy(allMail = kind, selected = view.messages.filter { m -> !m.sensitive }.map { m -> m.id }.toSet())
                }
            }
        }
    }
}

/** A choice of the sheet: the shared [dev.reins.android.design.SelectChip]. */
@Composable
private fun Chip(title: String, selected: Boolean, tag: String, onClick: () -> Unit) {
    dev.reins.android.design.SelectChip(title, selected, Modifier.testTag(tag), onClick = onClick)
}

@Composable
private fun ResourceRows(resources: List<ResourceView>, draft: ApprovalDraft, viewModel: ApprovalViewModel, pick: (ApprovalDraft) -> ApprovalDraft) {
    if (resources.isEmpty()) return
    FactGroup(
        rows = resources.map { r ->
            {
                CheckOption(r.id in draft.resources, untrusted(r.label), "resource:${r.id}") { on ->
                    viewModel.edit { d -> pick(d).copy(resources = toggleResource(d.resources, r.id, on)) }
                }
            }
        },
    )
}

@Composable
private fun CheckOption(checked: Boolean, title: String, tag: String, mono: Boolean = false, onChange: (Boolean) -> Unit) {
    val c = LocalColors.current
    val feedback = LocalFeedback.current
    Row(
        Modifier
            .fillMaxWidth()
            .toggleable(checked, role = Role.Checkbox) {
                onChange(it)
                feedback.play(Event.toggle(it))
            }
            .padding(horizontal = 16.dp, vertical = 13.dp)
            .testTag(tag),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        TickBox(checked)
        Spacer(Modifier.width(14.dp))
        RText(title, if (mono) RType.mono(14f) else RType.sans(15.5f), c.text, Modifier.weight(1f), maxLines = 2, ltr = mono)
    }
}

/** Gmail: only the ticked emails, or similar mail (senders, domains, subject); a send: each recipient, or its domain. */
@Composable
private fun GmailScope(view: ApprovalView, draft: ApprovalDraft, viewModel: ApprovalViewModel, pick: (ApprovalDraft) -> ApprovalDraft) {
    val c = LocalColors.current
    if (view.kind == ApprovalKind.SEND) {
        FactGroup(
            rows = recipientAddresses(view).map { address ->
                {
                    val wide = address in draft.domainRecipients
                    Row(Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 12.dp), verticalAlignment = Alignment.CenterVertically) {
                        Column(Modifier.weight(1f)) {
                            RText(address, RType.mono(14f), c.text, maxLines = 1, ltr = true)
                            RText(if (wide) "Anyone at @${domainOf(address)}" else "This address only", RType.sans(12.5f), c.tertiary, Modifier.padding(top = 2.dp))
                        }
                        Toggle(wide, Modifier.testTag("domain:$address")) { on ->
                            viewModel.edit { d -> pick(d).copy(domainRecipients = if (on) d.domainRecipients + address else d.domainRecipients - address) }
                        }
                    }
                }
            },
        )
    } else {
        val picked = !draft.likeThis || view.quick?.allow == null
        FactGroup(
            rows = buildList<@Composable () -> Unit> {
                add { CheckOption(picked && !draft.similar, "Only the emails I ticked", "onlySelected") { viewModel.edit { pick(it).copy(similar = false) } } }
                add { CheckOption(picked && draft.similar, "Also similar mail", "similar") { viewModel.edit { pick(it).copy(similar = true) } } }
                if (picked && draft.similar) {
                    val senders = senderAddresses(view, draft.selected)
                    senders.forEach { address ->
                        add {
                            CheckOption(address in draft.senderAddresses, address, "sender:$address", mono = true) { on ->
                                viewModel.edit { pick(it).copy(senderAddresses = if (on) it.senderAddresses + address else it.senderAddresses - address) }
                            }
                        }
                    }
                    senders.map(::domainOf).distinct().forEach { domain ->
                        add {
                            CheckOption(domain in draft.senderDomains, "Anyone at @$domain", "senderDomain:$domain") { on ->
                                viewModel.edit { pick(it).copy(senderDomains = if (on) it.senderDomains + domain else it.senderDomains - domain) }
                            }
                        }
                    }
                }
            },
        )
    }
    RTextField(
        draft.subject,
        { text -> viewModel.edit { pick(it).copy(subject = text.take(200)) } },
        "Subject contains (optional)",
        Modifier.padding(horizontal = 16.dp),
        tag = "subject",
    )
}

// ---- Autopilot ------------------------------------------------------------------------------------------------------

/** Autopilot's reasons: the verdict and how sure, the likelihoods, the decisions of yours it is like. */
@Composable
private fun AutopilotSheet(s: SuggestionView, onClose: () -> Unit, onSettings: () -> Unit) {
    val c = LocalColors.current
    val tint = if (s.judged && !s.floor) verdictTint(s.verdict, c) else c.secondary
    val p = if (s.verdict == Verdict.DENY) s.pDeny else s.pApprove
    Column(Modifier.testTag("suggestionDetail")) {
        SheetHead("Autopilot", onClose)
        Column(Modifier.weight(1f, fill = false).verticalScroll(rememberScrollState()).padding(horizontal = 22.dp)) {
            if (s.judged && !s.floor) {
                Row(verticalAlignment = Alignment.Bottom) {
                    RText(verdictLabel(s.verdict), RType.sans(30f, FontWeight.SemiBold), tint, maxLines = 1)
                    Spacer(Modifier.width(8.dp))
                    RText(AutopilotText.percent(p), RType.mono(24f, FontWeight.SemiBold), tint, Modifier.padding(bottom = 2.dp), maxLines = 1)
                }
            } else {
                RText(if (s.floor) "Always asked" else "Not scored", RType.sans(26f, FontWeight.SemiBold), tint, maxLines = 1)
            }
            if (s.reason.isNotBlank()) {
                RText(untrusted(s.reason), RType.sans(14.5f, lineHeight = 20f), c.secondary, Modifier.padding(top = 4.dp), maxLines = 3)
            }
            if (s.judged) {
                Column(Modifier.padding(top = 20.dp), verticalArrangement = Arrangement.spacedBy(12.dp)) {
                    Likelihood("Approve", s.pApprove, c.success)
                    Likelihood("Deny", s.pDeny, c.danger)
                    Likelihood("Confidence", s.confidence, c.accent)
                }
            }
            if (s.neighbours.isNotEmpty()) {
                RText(
                    "YOUR PAST DECISIONS",
                    RType.sans(12f, FontWeight.SemiBold).copy(letterSpacing = 1.sp),
                    c.tertiary,
                    Modifier.padding(top = 24.dp, bottom = 4.dp),
                )
                s.neighbours.take(4).forEachIndexed { i, n -> NeighbourRow(n, Modifier.testTag("neighbour:$i")) }
            }
            AutopilotText.suggestionNotes(s).forEach { note ->
                Row(Modifier.padding(top = 12.dp), verticalAlignment = Alignment.Top) {
                    Icon(Icons.Rounded.Info, null, Modifier.size(16.dp), tint = c.secondary)
                    Spacer(Modifier.width(8.dp))
                    RText(note, RType.sans(13.5f, lineHeight = 18f), c.secondary)
                }
            }
            Box(Modifier.padding(top = 14.dp).fillMaxWidth().height(1.dp).background(c.hairline))
            Row(
                Modifier
                    .fillMaxWidth()
                    .pressable(shape = RoundedCornerShape(8.dp), onClick = onSettings)
                    .padding(vertical = 14.dp)
                    .testTag("autopilotSettings"),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                RText("Autopilot settings", RType.sans(15f, FontWeight.Medium), c.accent, Modifier.weight(1f), maxLines = 1)
                Icon(Icons.Rounded.ChevronRight, null, Modifier.size(20.dp), tint = c.accent)
            }
        }
    }
}

@Composable
private fun Likelihood(label: String, p: Float, color: Color) {
    val c = LocalColors.current
    Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically) {
        RText(label, RType.sans(14f, FontWeight.Medium), c.secondary, Modifier.width(96.dp), maxLines = 1)
        MeterBar(p, color, Modifier.weight(1f))
        RText(AutopilotText.percent(p), RType.mono(14f, FontWeight.Medium), c.text, Modifier.width(48.dp), maxLines = 1, align = androidx.compose.ui.text.style.TextAlign.End)
    }
}

// ---- a permission's length -----------------------------------------------------------------------------------------

/** A permission the AI asked for can only be made shorter: a slider with a detent per step, up to what was asked. */
@Composable
private fun GrantSheet(view: ApprovalView, ui: ApprovalUi, viewModel: ApprovalViewModel, onClose: () -> Unit) {
    val c = LocalColors.current
    val asked = view.grant?.durationSecs?.toLong() ?: return
    val steps = listOf(60L, 600L, 3_600L, 6 * 3_600L, 86_400L, 7 * 86_400L, 30 * 86_400L).filter { it < asked } + asked
    val current = ui.draft.grantSeconds?.coerceAtMost(asked) ?: asked
    val index = steps.indexOfLast { it <= current }.coerceAtLeast(0)
    Column(Modifier.testTag("grantSheet")) {
        SheetHead("Allow for", onClose)
        Column(Modifier.padding(horizontal = 22.dp)) {
            RText(periodWords(steps[index]), RType.sans(26f, FontWeight.SemiBold), c.text, Modifier.testTag("grantDuration"))
            if (steps.size > 1) {
                Slider(
                    value = index.toFloat(),
                    onValueChange = detentAction(index, { it.toInt() }) { v -> viewModel.edit { it.copy(grantSeconds = steps[v.toInt().coerceIn(0, steps.lastIndex)]) } },
                    valueRange = 0f..steps.lastIndex.toFloat(),
                    steps = (steps.size - 2).coerceAtLeast(0),
                    modifier = Modifier.padding(top = 8.dp).testTag("grantSlider"),
                )
                RText("Asked for ${periodWords(asked)}. Only shorter.", RType.sans(13f), c.tertiary)
            }
            Row(
                Modifier
                    .padding(top = 18.dp)
                    .fillMaxWidth()
                    .height(54.dp)
                    .clip(CircleShape)
                    .background(c.text)
                    .pressable(shape = CircleShape, onClick = onClose)
                    .testTag("grantDone"),
                horizontalArrangement = Arrangement.Center,
                verticalAlignment = Alignment.CenterVertically,
            ) { RText("Done", RType.sans(16.5f, FontWeight.SemiBold), c.background, maxLines = 1) }
        }
    }
}
