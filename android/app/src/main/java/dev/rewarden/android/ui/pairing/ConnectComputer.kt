package dev.rewarden.android.ui.pairing

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
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
import dev.rewarden.android.design.Banner
import dev.rewarden.android.design.BannerKind
import dev.rewarden.android.design.ButtonStyle
import dev.rewarden.android.design.CapsuleButton
import dev.rewarden.android.design.Fonts
import dev.rewarden.android.design.Glyph
import dev.rewarden.android.design.Group
import dev.rewarden.android.design.ListRow
import dev.rewarden.android.design.LocalColors
import dev.rewarden.android.design.RTextField
import dev.rewarden.android.design.RType
import dev.rewarden.android.design.Screen
import dev.rewarden.android.platform.Browser
import dev.rewarden.android.platform.QrScannerProvider
import dev.rewarden.android.platform.ScanResult
import dev.rewarden.android.ui.AppViewModel
import dev.rewarden.android.ui.common.ReinsLinks
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
            append(" rewarden login ")
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
    val scan = rememberScan(app)
    var typed by rememberSaveable { mutableStateOf("") }
    Column(modifier.fillMaxWidth(), verticalArrangement = Arrangement.spacedBy(12.dp)) {
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
