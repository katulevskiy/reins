package dev.reins.android.ui.pairing

import androidx.compose.ui.semantics.semantics
import dev.reins.android.design.pressable
import androidx.compose.ui.draw.clip
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.border
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.windowInsetsPadding
import androidx.compose.foundation.verticalScroll

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.BasicText
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.SpanStyle
import androidx.compose.ui.text.buildAnnotatedString
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.KeyboardCapitalization
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.withStyle
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import dev.reins.android.design.Banner
import dev.reins.android.design.BannerKind
import dev.reins.android.design.ButtonStyle
import dev.reins.android.design.CapsuleButton
import dev.reins.android.design.Fonts
import dev.reins.android.design.Glyph
import dev.reins.android.design.Group
import dev.reins.android.design.ListRow
import dev.reins.android.design.LocalColors
import dev.reins.android.design.RText
import dev.reins.android.design.RTextField
import dev.reins.android.design.RType
import dev.reins.android.design.Screen
import dev.reins.android.platform.Browser
import dev.reins.android.platform.QrScannerProvider
import dev.reins.android.platform.ScanResult
import dev.reins.android.ui.AppViewModel
import dev.reins.android.ui.common.ReinsLinks
import dev.reins.android.ui.autopilot.IconTile
import dev.reins.android.ui.common.untrusted
import kotlinx.coroutines.launch

/** Settings > Connect > Computer: the scanner, or the code typed in. */
@Composable
fun ConnectComputerScreen(app: AppViewModel, onBack: () -> Unit) {
    // An error from this visit does not greet the next one.
    DisposableEffect(app) { onDispose { app.resetConnect() } }
    val c = LocalColors.current
    // Signal (iOS-Connect 02): a back button on paper, the serif title, what to do on the computer, then the scanner.
    Column(Modifier.fillMaxSize().background(c.paper).verticalScroll(androidx.compose.foundation.rememberScrollState())) {
        Box(Modifier.windowInsetsPadding(dev.reins.android.ui.main.HeaderInsets).padding(start = 24.dp, top = 8.dp)) {
            dev.reins.android.design.BackArrow(onBack, Modifier)
        }
        // Next (Pair and join 02): the title, the command to run on it, then the code typed or scanned.
        RText("Computer", RType.pageTitle, c.ink, Modifier.padding(start = 20.dp, end = 20.dp, top = 10.dp), maxLines = 1)
        ComputerHowTo(Modifier.padding(start = 16.dp, end = 16.dp, top = 14.dp))
        ConnectComputerPanel(app, Modifier.padding(horizontal = 16.dp, vertical = 16.dp))
        DesktopAppLink()
        Spacer(Modifier.height(32.dp))
    }
}

/** What to do on the computer: "Run  reins login" on a sunken row, the command in mono, with copy. */
@Composable
fun ComputerHowTo(modifier: Modifier = Modifier) {
    val c = LocalColors.current
    val context = LocalContext.current
    Row(
        modifier.fillMaxWidth().clip(androidx.compose.foundation.shape.RoundedCornerShape(16.dp)).background(c.paperRaised)
            .semantics(mergeDescendants = true) {}.testTag("computerHowTo").padding(start = 14.dp, end = 8.dp, top = 8.dp, bottom = 8.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        RText("Run  ", RType.sans(15f), c.ink2, maxLines = 1)
        RText("reins login", RType.mono(15f, FontWeight.Medium), c.ink, Modifier.weight(1f), maxLines = 1, ltr = true)
        Box(
            Modifier.size(40.dp).clip(androidx.compose.foundation.shape.CircleShape).border(1.dp, c.rule, androidx.compose.foundation.shape.CircleShape)
                .pressable(shape = androidx.compose.foundation.shape.CircleShape, label = "Copy reins login") {
                    context.getSystemService(android.content.ClipboardManager::class.java)?.setPrimaryClip(android.content.ClipData.newPlainText("Command", "reins login"))
                },
            contentAlignment = Alignment.Center,
        ) { dev.reins.android.design.GlyphIcon(Glyph.Copy, c.ink, size = 18.dp) }
    }
}

/**
 * The code as eight cells with a dash in the middle (Next "BCDF-GHJK"), over a text field that takes what is typed or
 * pasted; [PairingCode] reduces it.
 */
@Composable
private fun CodeCells(typed: String, enabled: Boolean, onChange: (String) -> Unit) {
    val c = LocalColors.current
    val letters = typed.uppercase().filter { it.isLetterOrDigit() }.take(8)
    androidx.compose.foundation.text.BasicTextField(
        typed,
        onChange,
        Modifier.fillMaxWidth().testTag("pairCode"),
        enabled = enabled,
        singleLine = true,
        textStyle = RType.mono(1f).copy(color = androidx.compose.ui.graphics.Color.Transparent),
        cursorBrush = androidx.compose.ui.graphics.SolidColor(androidx.compose.ui.graphics.Color.Transparent),
        keyboardOptions = KeyboardOptions(capitalization = KeyboardCapitalization.Characters, autoCorrectEnabled = false, keyboardType = KeyboardType.Ascii),
        decorationBox = { inner ->
            Box {
                Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.spacedBy(6.dp), verticalAlignment = Alignment.CenterVertically) {
                    for (i in 0 until 8) {
                        if (i == 4) RText("-", RType.mono(18f), c.inkFaint, maxLines = 1)
                        val on = i == letters.length
                        Box(
                            Modifier.weight(1f).height(56.dp).clip(androidx.compose.foundation.shape.RoundedCornerShape(14.dp)).background(c.paperRaised)
                                .border(if (on) 2.dp else 1.dp, if (on) c.violet else c.rule2, androidx.compose.foundation.shape.RoundedCornerShape(14.dp)),
                            contentAlignment = Alignment.Center,
                        ) { letters.getOrNull(i)?.let { RText(it.toString(), RType.mono(22f, FontWeight.Medium), c.ink, maxLines = 1) } }
                    }
                }
                Box(Modifier.size(1.dp)) { inner() }
            }
        },
    )
}

/**
 * "Scan QR code", and the field to type the code instead (opened by itself when the scanner cannot run). A code that
 * checks out opens the pairing sheet over whatever is showing.
 */
@Composable
fun ConnectComputerPanel(app: AppViewModel, modifier: Modifier = Modifier) {
    val ui by app.connect.collectAsStateWithLifecycle()
    val justPaired by app.justPaired.collectAsStateWithLifecycle()
    val scan = rememberScan(app)
    var typed by rememberSaveable { mutableStateOf("") }
    DisposableEffect(Unit) { onDispose { app.clearJustPaired() } }
    // Opened from Settings' "Scan QR code": the scanner starts at once.
    LaunchedEffect(ui.scanRequested) {
        if (ui.scanRequested) {
            app.scanStarted()
            scan()
        }
    }
    Column(modifier.fillMaxWidth(), verticalArrangement = Arrangement.spacedBy(12.dp)) {
        justPaired?.let { ConnectedCard(it) }
        ui.error?.let { Banner(it, kind = BannerKind.Error, tag = "connectError") }
        if (ui.manual) {
            CodeCells(typed, enabled = !ui.busy) { typed = it.take(MAX_TYPED) }
            dev.reins.android.design.SignalButton(
                "Continue",
                Modifier.fillMaxWidth().testTag("submitCode"),
                enabled = !ui.busy && PairingCode.normalize(typed) != null,
            ) { app.connectTyped(typed) }
            dev.reins.android.design.SignalButton(
                "Scan instead",
                Modifier.fillMaxWidth().testTag("scanQr"),
                style = dev.reins.android.design.ButtonStyle.Ghost,
                glyph = Glyph.QrCode,
                enabled = !ui.busy,
                onClick = scan,
            )
        } else {
            dev.reins.android.design.SignalButton(
                "Scan QR code",
                Modifier.fillMaxWidth().testTag("scanQr"),
                glyph = Glyph.QrCode,
                enabled = !ui.busy,
                onClick = scan,
            )
            dev.reins.android.design.SignalButton(
                "Type the code instead",
                Modifier.fillMaxWidth().testTag("typeCode"),
                style = dev.reins.android.design.ButtonStyle.Ghost,
                size = dev.reins.android.design.ButtonSize.Small,
                onClick = app::showCodeField,
            )
        }
    }
}

/** reins2fa.com/download, in the browser. */
@Composable
fun DesktopAppLink(modifier: Modifier = Modifier) {
    val context = LocalContext.current
    Group(modifier) {
        ListRow(
            "Get the desktop app",
            Modifier.testTag("desktopDownload"),
            subtitle = ReinsLinks.DOWNLOAD.removePrefix("https://"),
            glyph = Glyph.Laptop,
            ltrSubtitle = true,
            chevron = true,
        ) { Browser.open(context, ReinsLinks.DOWNLOAD) }
    }
}

/** Starts the scanner and hands what it read to [app]. */
@Composable
private fun rememberScan(app: AppViewModel): () -> Unit {
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    return remember<() -> Unit>(context, app, scope) {
        {
            scope.launch {
                when (val result = QrScannerProvider.factory(context).scan()) {
                    is ScanResult.Scanned -> app.connectScanned(result.text)
                    ScanResult.Cancelled -> Unit
                    is ScanResult.Unavailable -> app.scannerUnavailable()
                }
            }
        }
    }
}

/** "BCDF-GHJK" with room for spaces. */
private const val MAX_TYPED = 20

/** The computer that just paired: the result of this page, before anything else on it. */
@Composable
private fun ConnectedCard(name: String) {
    val c = LocalColors.current
    Row(
        Modifier
            .fillMaxWidth()
            .background(c.success.copy(alpha = 0.12f), RoundedCornerShape(18.dp))
            .padding(14.dp)
            .testTag("computerConnected"),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        IconTile(Glyph.Check, c.success, size = 30.dp, filled = true)
        Spacer(Modifier.width(12.dp))
        Column(Modifier.weight(1f)) {
            RText("Connected", RType.sans(15.5f, FontWeight.SemiBold), c.text)
            RText(untrusted(name), RType.sans(13.5f), c.secondary, maxLines = 1)
        }
    }
}
