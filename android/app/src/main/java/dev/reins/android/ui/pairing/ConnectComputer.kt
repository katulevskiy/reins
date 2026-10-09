package dev.reins.android.ui.pairing

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

/** Settings > AI connections > Connect a computer. */
@Composable
fun ConnectComputerScreen(app: AppViewModel, onBack: () -> Unit) {
    // An error from this visit does not greet the next one.
    DisposableEffect(app) { onDispose { app.resetConnect() } }
    Screen(title = "Connect a computer", onBack = onBack) {
        ComputerHowTo(Modifier.padding(start = 24.dp, end = 24.dp, top = 12.dp))
        ConnectComputerPanel(app, Modifier.padding(horizontal = 16.dp, vertical = 18.dp))
        DesktopAppLink()
        Spacer(Modifier.height(32.dp))
    }
}

/** What to do on the computer, with the command set in mono. */
@Composable
fun ComputerHowTo(modifier: Modifier = Modifier) {
    val c = LocalColors.current
    val text = buildAnnotatedString {
        append("Run ")
        withStyle(SpanStyle(fontFamily = Fonts.mono, fontWeight = FontWeight.Medium, color = c.text, background = c.controlFill)) {
            append(" reins login ")
        }
        append(" on your computer (or open the Reins desktop app) and scan the QR code it shows.")
    }
    BasicText(text, modifier.testTag("computerHowTo"), RType.sans(15.5f, lineHeight = 22f).copy(color = c.secondary))
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
    Column(modifier.fillMaxWidth(), verticalArrangement = Arrangement.spacedBy(12.dp)) {
        justPaired?.let { ConnectedCard(it) }
        CapsuleButton(
            "Scan QR code",
            Modifier.fillMaxWidth().testTag("scanQr"),
            style = ButtonStyle.Accent,
            glyph = Glyph.Qr,
            enabled = !ui.busy,
            busy = ui.busy && !ui.manual,
            onClick = scan,
        )
        ui.error?.let { Banner(it, kind = BannerKind.Error, tag = "connectError") }
        if (ui.manual) {
            RTextField(
                typed,
                { typed = it.take(MAX_TYPED) },
                "BCDF-GHJK",
                tag = "pairCode",
                enabled = !ui.busy,
                mono = true,
                keyboardOptions = KeyboardOptions(
                    capitalization = KeyboardCapitalization.Characters,
                    autoCorrectEnabled = false,
                    keyboardType = KeyboardType.Ascii,
                ),
            )
            CapsuleButton(
                "Connect",
                Modifier.fillMaxWidth().testTag("submitCode"),
                style = ButtonStyle.Secondary,
                enabled = !ui.busy && PairingCode.normalize(typed) != null,
                busy = ui.busy,
            ) { app.connectTyped(typed) }
        } else {
            CapsuleButton(
                "Type the code instead",
                Modifier.fillMaxWidth().testTag("typeCode"),
                style = ButtonStyle.Ghost,
                compact = true,
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
