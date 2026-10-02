package dev.rewarden.android.ui.signin

import android.Manifest
import android.content.ClipData
import android.content.ClipboardManager
import android.content.Context
import android.content.pm.PackageManager
import android.os.Build
import androidx.activity.compose.BackHandler
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.shape.RoundedCornerShape
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
import androidx.compose.ui.unit.sp
import dev.rewarden.android.design.ButtonStyle
import dev.rewarden.android.design.CapsuleButton
import dev.rewarden.android.design.Glyph
import dev.rewarden.android.design.GlyphIcon
import dev.rewarden.android.design.Group
import dev.rewarden.android.design.LocalColors
import dev.rewarden.android.design.RText
import dev.rewarden.android.design.RType
import dev.rewarden.android.design.Screen
import dev.rewarden.android.feedback.Cue
import dev.rewarden.android.feedback.Event
import dev.rewarden.android.feedback.LocalFeedback
import dev.rewarden.android.feedback.cueUnlessRecent
import dev.rewarden.android.feedback.play
import dev.rewarden.android.ui.AppViewModel
import dev.rewarden.android.ui.pairing.ComputerHowTo
import dev.rewarden.android.ui.pairing.ConnectComputerPanel
import dev.rewarden.android.ui.pairing.DesktopAppLink

/** The two pages after a fresh sign-in. */
enum class SetupPage { Computer, Ai }

/**
 * Right after signing in or creating an account (once per account): notifications, then "connect your computer", then
 * "connect Claude.ai or ChatGPT". Skip or Done leads to the main screen. Sheets (a pairing found by its code) open over
 * it like over any screen.
 */
@Composable
fun SetupScreen(app: AppViewModel, serverUrl: String) {
    val feedback = LocalFeedback.current
    var page by rememberSaveable { mutableStateOf(SetupPage.Computer) }
    AskForNotificationsOnce()
    val back = {
        feedback.cueUnlessRecent(Cue.Close)
        page = SetupPage.Computer
    }
    BackHandler(enabled = page == SetupPage.Ai, onBack = back)
    when (page) {
        SetupPage.Computer -> ComputerPage(
            app,
            onNext = {
                feedback.cue(Cue.Open)
                app.resetConnect()
                page = SetupPage.Ai
            },
            onSkip = app::finishSetup,
        )
        SetupPage.Ai -> AiPage(AccountRules.mcpUrl(serverUrl), onBack = back, onDone = app::finishSetup)
    }
}

/** Approval requests arrive as notifications; Android 13 and later ask for that once. */
@Composable
private fun AskForNotificationsOnce() {
    val context = LocalContext.current
    var asked by rememberSaveable { mutableStateOf(false) }
    val launcher = rememberLauncherForActivityResult(ActivityResultContracts.RequestPermission()) { }
    LaunchedEffect(Unit) {
        if (asked) return@LaunchedEffect
        asked = true
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU &&
            context.checkSelfPermission(Manifest.permission.POST_NOTIFICATIONS) != PackageManager.PERMISSION_GRANTED
        ) {
            launcher.launch(Manifest.permission.POST_NOTIFICATIONS)
        }
    }
}

@Composable
private fun ComputerPage(app: AppViewModel, onNext: () -> Unit, onSkip: () -> Unit) {
    Screen(title = null, modifier = Modifier.testTag("setupComputer")) {
        SetupHeader(step = 1, glyph = Glyph.Laptop, title = "Next: connect your computer")
        ComputerHowTo(Modifier.padding(start = 24.dp, end = 24.dp, top = 10.dp))
        ConnectComputerPanel(app, Modifier.padding(horizontal = 16.dp, vertical = 20.dp))
        DesktopAppLink()
        Row(Modifier.fillMaxWidth().padding(start = 16.dp, end = 16.dp, top = 28.dp, bottom = 24.dp), horizontalArrangement = Arrangement.spacedBy(12.dp)) {
            CapsuleButton("Skip setup", Modifier.weight(1f).testTag("setupSkip"), style = ButtonStyle.Ghost, onClick = onSkip)
            CapsuleButton("Next", Modifier.weight(1f).testTag("setupNext"), style = ButtonStyle.Primary, onClick = onNext)
        }
    }
}

@Composable
private fun AiPage(mcpUrl: String, onBack: () -> Unit, onDone: () -> Unit) {
    val c = LocalColors.current
    val context = LocalContext.current
    val feedback = LocalFeedback.current
    var copied by rememberSaveable { mutableStateOf(false) }
    Screen(title = null, modifier = Modifier.testTag("setupAi")) {
        SetupHeader(step = 2, glyph = Glyph.Sparkle, title = "Connect Claude.ai or ChatGPT")
        RText(
            "Add Reins to your AI app with this address. Whatever it then wants to read or send asks this phone first.",
            RType.sans(15.5f, lineHeight = 22f),
            c.secondary,
            Modifier.padding(start = 24.dp, end = 24.dp, top = 10.dp),
        )
        Group(Modifier.padding(top = 18.dp)) {
            Row(Modifier.fillMaxWidth().padding(start = 16.dp, end = 10.dp, top = 12.dp, bottom = 12.dp), verticalAlignment = Alignment.CenterVertically) {
                RText(mcpUrl, RType.mono(15f, FontWeight.Medium), c.text, Modifier.weight(1f).testTag("mcpUrl"), ltr = true)
                Spacer(Modifier.width(10.dp))
                CapsuleButton(
                    if (copied && Build.VERSION.SDK_INT < Build.VERSION_CODES.TIRAMISU) "Copied" else "Copy",
                    Modifier.testTag("copyMcp"),
                    style = ButtonStyle.Secondary,
                    glyph = Glyph.Copy,
                    compact = true,
                ) {
                    copyText(context, "Reins MCP address", mcpUrl)
                    // Android 13 and later confirm a copy themselves.
                    copied = true
                    feedback.play(Event.Copied)
                }
            }
        }
        RText(
            "In Claude.ai or ChatGPT, open Settings > Connectors, add a custom connector and paste this address.",
            RType.sans(13.5f, lineHeight = 19f),
            c.tertiary,
            Modifier.padding(start = 32.dp, end = 32.dp, top = 10.dp).testTag("connectorHowTo"),
        )
        Row(Modifier.fillMaxWidth().padding(start = 16.dp, end = 16.dp, top = 28.dp, bottom = 24.dp), horizontalArrangement = Arrangement.spacedBy(12.dp)) {
            CapsuleButton("Back", Modifier.weight(1f).testTag("setupBack"), style = ButtonStyle.Ghost, onClick = onBack)
            CapsuleButton("Done", Modifier.weight(1f).testTag("setupDone"), style = ButtonStyle.Primary, onClick = onDone)
        }
    }
}

/** "Step n of 2", the page's glyph on a soft tile, and its title. */
@Composable
private fun SetupHeader(step: Int, glyph: Glyph, title: String) {
    val c = LocalColors.current
    Column(Modifier.padding(start = 24.dp, end = 24.dp, top = 40.dp)) {
        RText(
            "STEP $step OF 2",
            RType.sans(12.5f, FontWeight.Medium).copy(letterSpacing = 0.6.sp),
            c.secondary,
            Modifier.testTag("setupStep"),
        )
        Spacer(Modifier.height(16.dp))
        Box(Modifier.size(56.dp).background(c.accentSoft, RoundedCornerShape(18.dp)), contentAlignment = Alignment.Center) {
            GlyphIcon(glyph, c.accent, size = 30.dp)
        }
        Spacer(Modifier.height(14.dp))
        RText(title, RType.sans(28f, FontWeight.SemiBold, lineHeight = 34f), c.text)
    }
}

private fun copyText(context: Context, label: String, text: String) {
    context.getSystemService(ClipboardManager::class.java)?.setPrimaryClip(ClipData.newPlainText(label, text))
}
