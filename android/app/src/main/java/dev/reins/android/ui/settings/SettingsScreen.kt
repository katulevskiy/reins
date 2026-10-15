package dev.reins.android.ui.settings

import android.content.ClipData
import android.content.ClipboardManager
import android.content.Context
import android.os.Build
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ExperimentalLayoutApi
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.rounded.Logout
import androidx.compose.material.icons.automirrored.rounded.VolumeUp
import androidx.compose.material.icons.rounded.Add
import androidx.compose.material.icons.rounded.Bolt
import androidx.compose.material.icons.rounded.ContentCopy
import androidx.compose.material.icons.rounded.Delete
import androidx.compose.material.icons.rounded.Devices
import androidx.compose.material.icons.rounded.Explore
import androidx.compose.material.icons.rounded.Extension
import androidx.compose.material.icons.rounded.Fingerprint
import androidx.compose.material.icons.rounded.Key
import androidx.compose.material.icons.rounded.Laptop
import androidx.compose.material.icons.rounded.Lock
import androidx.compose.material.icons.rounded.Notifications
import androidx.compose.material.icons.rounded.NotificationsOff
import androidx.compose.material.icons.rounded.HowToReg
import androidx.compose.material.icons.rounded.PhoneAndroid
import androidx.compose.material.icons.rounded.QrCodeScanner
import androidx.compose.material.icons.rounded.WarningAmber
import androidx.compose.material3.Icon
import androidx.compose.runtime.Composable
import androidx.compose.runtime.setValue
import androidx.compose.runtime.getValue
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import dev.reins.android.BuildConfig
import dev.reins.android.design.Banner
import dev.reins.android.design.BannerKind
import dev.reins.android.design.BlobAvatar
import dev.reins.android.design.ConfirmDialog
import dev.reins.android.design.LocalColors
import dev.reins.android.design.RText
import dev.reins.android.design.RType
import dev.reins.android.design.pressable
import dev.reins.android.feedback.Event
import dev.reins.android.feedback.FeedbackSettings
import dev.reins.android.feedback.LocalFeedback
import dev.reins.android.feedback.play
import dev.reins.android.platform.Authenticator
import dev.reins.android.platform.Browser
import dev.reins.android.platform.FirebaseSupport
import dev.reins.android.platform.rememberNotificationState
import dev.reins.android.state.AppState
import dev.reins.android.state.SessionState
import dev.reins.android.ui.common.ConnectionIcon
import dev.reins.android.ui.common.ReinsLinks
import dev.reins.android.ui.common.relativeTime
import dev.reins.android.ui.common.untrusted
import dev.reins.android.ui.signin.AccountRules
import dev.reins.core.AutopilotMode
import dev.reins.core.ConnectionView

/**
 * Settings: the account card (email, server, whether this phone approves and is notified, with the one fix when
 * something is off), what is connected, Autopilot and the other pages, security, the tour and version, and leaving.
 */
@Composable
fun SettingsScreen(
    state: AppState,
    viewModel: SettingsViewModel,
    /** Null at the root of the Settings tab. */
    onBack: (() -> Unit)? = null,
    onConnection: (String) -> Unit,
    onIntegrations: () -> Unit,
    onSounds: () -> Unit,
    onTour: () -> Unit = {},
    onAutopilot: () -> Unit = {},
    onScanComputer: () -> Unit = {},
    onTypeComputerCode: () -> Unit = {},
    onVaultPasskeys: () -> Unit = {},
    onDevices: () -> Unit = {},
    /** Debug builds: the Signal component gallery. */
    onSignalGallery: () -> Unit = {},
    /** A sheet covers the page: the tab bar steps aside for it. */
    onCovered: (Boolean) -> Unit = {},
    authenticator: Authenticator,
) {
    val c = LocalColors.current
    val feedback = LocalFeedback.current
    val context = LocalContext.current
    val ui by viewModel.ui.collectAsStateWithLifecycle()
    val sounds by viewModel.sounds.collectAsStateWithLifecycle()
    val session by state.session.collectAsStateWithLifecycle()
    val connections by state.connections.collectAsStateWithLifecycle()
    val accounts by state.accounts.collectAsStateWithLifecycle()
    val autopilot by state.autopilot.collectAsStateWithLifecycle()
    val hasRecoveryCode by viewModel.hasRecoveryCode.collectAsStateWithLifecycle()
    val recoveryCode by viewModel.recoveryCode.collectAsStateWithLifecycle()
    val deletion by viewModel.deletion.collectAsStateWithLifecycle()
    val vaultPasskeys by viewModel.vaultPasskeys.collectAsStateWithLifecycle()
    val devices by viewModel.devices.collectAsStateWithLifecycle()
    var confirmSignOut by remember { mutableStateOf(false) }
    var connectOpen by remember { mutableStateOf(false) }
    LaunchedEffect(connectOpen) { onCovered(connectOpen) }
    LaunchedEffect(session) {
        viewModel.checkRecoveryCode()
        viewModel.countDevices()
    }
    val signedIn = session as? SessionState.SignedIn
    val mcpUrl = signedIn?.info?.serverUrl?.let(AccountRules::mcpUrl)
    val copyMcp: () -> Unit = {
        if (mcpUrl != null) {
            copyText(context, "Reins MCP address", mcpUrl)
            feedback.play(Event.Copied)
            // Android 13 and later confirm a copy themselves.
            if (Build.VERSION.SDK_INT < Build.VERSION_CODES.TIRAMISU) viewModel.notice("MCP address copied.")
        }
    }

    Box(Modifier.fillMaxSize()) {
        SettingsPage(title = "Settings", onBack = onBack) {
            signedIn?.let { AccountCard(it.info.email, it.info.serverUrl, state, viewModel) }
            Column(Modifier.padding(horizontal = 16.dp)) {
                ui.message?.let { Banner(it, Modifier.padding(top = 10.dp)) }
                ui.error?.let { Banner(it, Modifier.padding(top = 10.dp), BannerKind.Error) }
            }

            SettingsGroup(header = "Connected") {
                // AI apps first, then the computers, which pinned their key when they paired.
                connections.sortedBy { it.keyFingerprint != null }.forEach { connection ->
                    ConnectionRow(connection) { onConnection(connection.id) }
                }
                SettingsRow(
                    "Connect",
                    Modifier.testTag("connect"),
                    icon = Icons.Rounded.Add,
                    tint = c.accent,
                    titleColor = c.accent,
                    chevron = false,
                ) { connectOpen = true }
            }

            SettingsGroup {
                val mode = autopilot?.mode ?: AutopilotMode.MANUAL
                SettingsRow(
                    "Autopilot",
                    Modifier.testTag("openAutopilot"),
                    icon = Icons.Rounded.Bolt,
                    tint = c.accent,
                    valueContent = { ModeValue(mode) },
                    onClick = onAutopilot,
                )
                SettingsRow(
                    "Integrations",
                    Modifier.testTag("openIntegrations"),
                    icon = Icons.Rounded.Extension,
                    tint = c.read,
                    // The vault comes with the account: it is not something the user connected.
                    value = accountsValue(accounts.count { it.service != "vault" }),
                    onClick = onIntegrations,
                )
                SettingsRow(
                    "Sounds & haptics",
                    Modifier.testTag("openSounds"),
                    icon = Icons.AutoMirrored.Rounded.VolumeUp,
                    tint = c.accounts,
                    value = soundsValue(sounds),
                    onClick = onSounds,
                )
            }

            if (signedIn != null) {
                SettingsGroup(header = "Security") {
                    SettingsRow(
                        "Devices",
                        Modifier.testTag("devicesRow"),
                        icon = Icons.Rounded.Devices,
                        tint = c.search,
                        value = devices?.toString(),
                        onClick = onDevices,
                    )
                    if (hasRecoveryCode) {
                        SettingsRow(
                            "Recovery code",
                            Modifier.testTag("recoveryCodeRow"),
                            icon = Icons.Rounded.Key,
                            tint = c.send,
                        ) { viewModel.showRecoveryCode(authenticator) }
                        SettingsRow(
                            "Vault passkeys",
                            Modifier.testTag("vaultPasskeysRow"),
                            icon = Icons.Rounded.Fingerprint,
                            tint = c.accent,
                            value = vaultPasskeysValue(vaultPasskeys),
                            onClick = onVaultPasskeys,
                        )
                    }
                }
            }

            SettingsGroup {
                SettingsRow(
                    "Take the tour",
                    Modifier.testTag("takeTour"),
                    icon = Icons.Rounded.Explore,
                    tint = c.tertiary,
                    onClick = onTour,
                )
                VersionRow()
                val updates = viewModel.updates
                if (updates != null) {
                    val update by updates.state.collectAsStateWithLifecycle()
                    UpdateRows(
                        update,
                        onCheck = updates::checkNow,
                        onInstall = updates::install,
                        onRetry = updates::retry,
                        onAutoDownload = updates::setAutoDownload,
                    )
                }
            }

            SettingsGroup {
                SettingsRow(
                    "Sign out",
                    Modifier.testTag("signOut"),
                    icon = Icons.AutoMirrored.Rounded.Logout,
                    destructive = true,
                    chevron = false,
                    onClick = { if (!ui.busy) confirmSignOut = true },
                )
                if (signedIn != null) {
                    SettingsRow(
                        "Delete account",
                        Modifier.testTag("deleteAccount"),
                        icon = Icons.Rounded.Delete,
                        destructive = true,
                        chevron = false,
                        onClick = viewModel::openDeletion,
                    )
                }
            }

            Footer()

            if (BuildConfig.DEBUG) {
                // Debug builds only: what the round Autopilot button opens, to compare the designs on a phone.
                SettingsGroup(header = "Developer") {
                    val quick by dev.reins.android.ui.autopilot.QuickStyles.state(context)
                    dev.reins.android.ui.autopilot.Segmented(
                        dev.reins.android.ui.autopilot.QuickStyle.entries,
                        quick,
                        label = { it.label },
                        modifier = Modifier.padding(12.dp),
                        tag = { "quickStyle:${it.name}" },
                    ) { dev.reins.android.ui.autopilot.QuickStyles.set(context, it) }
                    SettingsRow("Signal gallery", Modifier.testTag("signalGalleryRow"), onClick = onSignalGallery)
                }
            }
            Spacer(Modifier.height(32.dp))
        }

        BottomSheet(connectOpen, "Connect", onClose = { connectOpen = false }, tag = "connectSheet") {
            ConnectSheetContent(
                mcpUrl = mcpUrl,
                onCopy = copyMcp,
                onScan = {
                    connectOpen = false
                    onScanComputer()
                },
                onType = {
                    connectOpen = false
                    onTypeComputerCode()
                },
            )
        }
    }

    recoveryCode?.let { code ->
        RecoveryCodeSheet(
            code,
            onCopy = {
                copySecret(context, "Reins recovery code", code)
                feedback.play(Event.Copied)
            },
            onDone = viewModel::hideRecoveryCode,
        )
    }

    val signedInEmail = signedIn?.info?.email
    deletion?.let { d ->
        if (signedInEmail != null) {
            DeleteAccountSheet(signedInEmail, d, onDelete = viewModel::deleteAccount, onDismiss = viewModel::closeDeletion)
        }
    }

    if (confirmSignOut) {
        ConfirmDialog(
            title = "Sign out?",
            text = "Requests can't reach this phone until you sign in again.",
            confirmLabel = "Sign out",
            onConfirm = {
                confirmSignOut = false
                viewModel.signOut()
            },
            onDismiss = { confirmSignOut = false },
        )
    }
}

/**
 * Who is signed in and to which server (a tap copies it), then whether this phone approves and is notified: green
 * chips when all is well, or the one thing to do about it.
 */
@OptIn(ExperimentalLayoutApi::class)
@Composable
private fun AccountCard(email: String, serverUrl: String, state: AppState, viewModel: SettingsViewModel) {
    val c = LocalColors.current
    val feedback = LocalFeedback.current
    val context = LocalContext.current
    val ui by viewModel.ui.collectAsStateWithLifecycle()
    val approvalDevice by state.approvalDevice.collectAsStateWithLifecycle()
    val serverPush by viewModel.serverPush.collectAsStateWithLifecycle()
    val notifications = rememberNotificationState()
    val push = pushChip(approvalDevice, notifications.enabled, appPushOverride ?: FirebaseSupport.available(context), serverPush)
    val shape = RoundedCornerShape(28.dp)
    Column(
        Modifier
            .padding(horizontal = 16.dp)
            .fillMaxWidth()
            .background(c.accent.copy(alpha = 0.09f), shape)
            .border(1.dp, c.accent.copy(alpha = 0.24f), shape)
            .padding(18.dp)
            .testTag("accountCard"),
    ) {
        Row(verticalAlignment = Alignment.CenterVertically) {
            BlobAvatar(email, size = 52.dp)
            Spacer(Modifier.width(14.dp))
            Column(Modifier.weight(1f)) {
                RText(email, RType.sans(17f, FontWeight.SemiBold), c.text, Modifier.testTag("accountEmail"), maxLines = 1, ltr = true)
                Row(
                    Modifier
                        .padding(top = 3.dp)
                        .pressable(shape = RoundedCornerShape(6.dp), label = "Copy server address") {
                            copyText(context, "Reins server", serverUrl)
                            feedback.play(Event.Copied)
                            // Android 13 and later confirm a copy themselves.
                            if (Build.VERSION.SDK_INT < Build.VERSION_CODES.TIRAMISU) viewModel.notice("Server address copied.")
                        }
                        .testTag("copyServer"),
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    RText(hostOf(serverUrl), RType.mono(13f), c.secondary, Modifier.weight(1f, fill = false), maxLines = 1, ltr = true)
                    Spacer(Modifier.width(6.dp))
                    Icon(Icons.Rounded.ContentCopy, null, Modifier.size(13.dp), tint = c.tertiary)
                }
            }
        }
        FlowRow(Modifier.padding(top = 16.dp), horizontalArrangement = Arrangement.spacedBy(8.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
            if (approvalDevice) {
                StatusChip("Approval phone", Icons.Rounded.PhoneAndroid, ChipTone.Ok, Modifier.testTag("approvalChip"))
            } else {
                StatusChip("Another phone approves", Icons.Rounded.PhoneAndroid, ChipTone.Warn, Modifier.testTag("approvalChip"))
            }
            when (push) {
                PushChip.On -> StatusChip("Notifications on", Icons.Rounded.Notifications, ChipTone.Ok, Modifier.testTag("notificationsChip"))
                PushChip.OnlyWhileOpen -> StatusChip("Only while open", Icons.Rounded.NotificationsOff, ChipTone.Plain, Modifier.testTag("notificationsChip"))
                PushChip.Hidden -> Unit
            }
        }
        if (!notifications.enabled) {
            Row(
                Modifier
                    .padding(top = 14.dp)
                    .fillMaxWidth()
                    .background(c.warning.copy(alpha = 0.08f), RoundedCornerShape(16.dp))
                    .border(1.dp, c.warning.copy(alpha = 0.22f), RoundedCornerShape(16.dp))
                    .padding(start = 14.dp, end = 10.dp, top = 10.dp, bottom = 10.dp)
                    .testTag("notificationsRow"),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                Icon(Icons.Rounded.WarningAmber, null, Modifier.size(18.dp), tint = c.warning)
                Spacer(Modifier.width(12.dp))
                RText("Notifications are off", RType.sans(14.5f, FontWeight.Medium), c.text, Modifier.weight(1f), maxLines = 2)
                PillButton(
                    "Turn on",
                    Modifier.testTag("turnOnNotifications"),
                    background = c.warning,
                    foreground = if (c.dark) Color(0xFF231A00) else Color.White,
                    height = 36.dp,
                    textSize = 14f,
                ) { notifications.request() }
            }
        }
        if (!approvalDevice) {
            val (bg, fg) = primaryFill(c)
            PillButton(
                "Use this phone for approvals",
                Modifier.padding(top = 14.dp).fillMaxWidth().testTag("registerPhone"),
                icon = Icons.Rounded.PhoneAndroid,
                background = bg,
                foreground = fg,
                height = 50.dp,
                textSize = 16f,
                enabled = !ui.busy,
                busy = ui.busy,
                onClick = viewModel::registerThisPhone,
            )
        }
    }
}

/** A connected AI app or computer: its icon, its name, and where it connects from and when it was last used. */
@Composable
private fun ConnectionRow(connection: ConnectionView, onClick: () -> Unit) {
    val c = LocalColors.current
    SettingsRow(
        untrusted(connection.label),
        Modifier.testTag("connection:${connection.id}"),
        subtitle = connectionLine(connection),
        subtitleTag = "connectionDetail:${connection.id}",
        leading = {
            if (connection.keyFingerprint != null && connection.icon == null) {
                IconPlate(Icons.Rounded.Laptop, c.pair)
            } else {
                ConnectionIcon(connection.id, connection.label, size = 40.dp)
            }
        },
        onClick = onClick,
    )
}

/** Autopilot's mode on the right of its row: the mode's icon in its colour, and its name. */
@Composable
private fun ModeValue(mode: AutopilotMode) {
    val c = LocalColors.current
    Row(verticalAlignment = Alignment.CenterVertically, modifier = Modifier.testTag("autopilotMode")) {
        Icon(settingsModeIcon(mode), null, Modifier.size(16.dp), tint = dev.reins.android.ui.autopilot.modeTint(mode, c))
        Spacer(Modifier.width(5.dp))
        RText(dev.reins.android.autopilot.AutopilotText.name(mode), RType.sans(15f), c.secondary, maxLines = 1)
    }
}

/** The connect sheet: an AI app by the address to paste (copied here), or a computer by the code it shows. */
@Composable
private fun ConnectSheetContent(mcpUrl: String?, onCopy: () -> Unit, onScan: () -> Unit, onType: () -> Unit) {
    val c = LocalColors.current
    var help by remember { mutableStateOf(false) }
    val card = Modifier
        .padding(horizontal = 16.dp, vertical = 5.dp)
        .fillMaxWidth()
        .background(if (c.dark) c.elevated else c.background, RoundedCornerShape(22.dp))
        .border(1.dp, if (c.dark) Color(0xFF1F1F23) else c.hairline, RoundedCornerShape(22.dp))
        .padding(16.dp)
    Column(card.testTag("connectAi")) {
        Row(verticalAlignment = Alignment.CenterVertically) {
            Box(Modifier.width(58.dp)) {
                Box(Modifier.padding(start = 24.dp)) { dev.reins.android.design.ConnectionAvatar("ChatGPT", "openai", size = 34.dp) }
                Box(Modifier.border(2.5.dp, if (c.dark) c.elevated else c.background, CircleShape)) {
                    dev.reins.android.design.ConnectionAvatar("Claude", "claude", size = 34.dp)
                }
            }
            Spacer(Modifier.width(10.dp))
            Column(Modifier.weight(1f)) {
                RText("AI app", RType.sans(16.5f, FontWeight.SemiBold), c.text, maxLines = 1)
                RText("Claude, ChatGPT and others", RType.sans(13f), c.tertiary, Modifier.padding(top = 1.dp), maxLines = 1)
            }
            HelpIcon(help, "AI apps") { help = it }
        }
        dev.reins.android.design.HelpText(
            "In Claude.ai, ChatGPT or another AI app, add a custom connector (MCP) and paste this address. It then asks you here before it reads or sends anything.",
            help,
            Modifier.padding(top = 10.dp),
        )
        if (mcpUrl != null) {
            Row(
                Modifier
                    .padding(top = 14.dp)
                    .fillMaxWidth()
                    .background(if (c.dark) Color(0xFF060606) else c.elevated, RoundedCornerShape(13.dp))
                    .pressable(shape = RoundedCornerShape(13.dp), label = "Copy MCP address", onClick = onCopy)
                    .padding(start = 14.dp, end = 12.dp, top = 11.dp, bottom = 11.dp)
                    .testTag("copyMcpUrl"),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                RText(mcpUrl.removePrefix("https://"), RType.mono(13f), c.text, Modifier.weight(1f), maxLines = 1, ltr = true)
                Spacer(Modifier.width(10.dp))
                Icon(Icons.Rounded.ContentCopy, null, Modifier.size(16.dp), tint = c.accent)
                Spacer(Modifier.width(6.dp))
                RText("Copy", RType.sans(14f, FontWeight.SemiBold), c.accent, maxLines = 1)
            }
        }
    }
    Column(card.testTag("connectComputerCard")) {
        Row(verticalAlignment = Alignment.CenterVertically) {
            IconPlate(Icons.Rounded.Laptop, c.pair)
            Spacer(Modifier.width(12.dp))
            Column(Modifier.weight(1f)) {
                RText("Computer", RType.sans(16.5f, FontWeight.SemiBold), c.text, maxLines = 1)
                Row(Modifier.padding(top = 1.dp)) {
                    RText("Desktop app or ", RType.sans(13f), c.tertiary, maxLines = 1)
                    RText("reins login", RType.mono(13f), c.tertiary, maxLines = 1, ltr = true)
                }
            }
        }
        Row(Modifier.padding(top = 14.dp), horizontalArrangement = Arrangement.spacedBy(10.dp)) {
            val (bg, fg) = primaryFill(c)
            PillButton(
                "Scan QR code",
                Modifier.weight(1f).testTag("sheetScanQr"),
                icon = Icons.Rounded.QrCodeScanner,
                background = bg,
                foreground = fg,
                onClick = onScan,
            )
            PillButton("Type code", Modifier.testTag("sheetTypeCode"), background = c.controlFill, foreground = c.text, onClick = onType)
        }
    }
}

/** "Reins 1.4 · Terms · Privacy", small and centred under the last group. */
@Composable
private fun Footer() {
    val c = LocalColors.current
    val context = LocalContext.current
    Row(Modifier.fillMaxWidth().padding(top = 18.dp), horizontalArrangement = Arrangement.Center, verticalAlignment = Alignment.CenterVertically) {
        RText("Reins ${BuildConfig.VERSION_NAME}  ·  ", RType.sans(12.5f), c.tertiary, maxLines = 1)
        RText(
            "Terms",
            RType.sans(12.5f),
            c.secondary,
            Modifier.pressable(shape = RoundedCornerShape(6.dp), label = "Terms") { Browser.open(context, ReinsLinks.TERMS) }.testTag("terms").padding(vertical = 6.dp),
            maxLines = 1,
        )
        RText("  ·  ", RType.sans(12.5f), c.tertiary, maxLines = 1)
        RText(
            "Privacy",
            RType.sans(12.5f),
            c.secondary,
            Modifier.pressable(shape = RoundedCornerShape(6.dp), label = "Privacy") { Browser.open(context, ReinsLinks.PRIVACY) }.testTag("privacy").padding(vertical = 6.dp),
            maxLines = 1,
        )
    }
}

/** Design-review screenshots stand in for a build with push (Robolectric has no Firebase); null in the app. */
internal var appPushOverride: Boolean? = null

/** How the account card shows notifications: on, reaching this phone only while Reins is open, or not at all. */
internal enum class PushChip { On, OnlyWhileOpen, Hidden }

/**
 * The notification chip of the account card. Only the approval phone gets requests; with notifications off the card
 * says so instead (and offers to turn them on). Without push in this build ([appPush]) or on the server
 * ([serverPush] false; null: it does not say), requests arrive only while Reins is open.
 */
internal fun pushChip(approvalDevice: Boolean, notificationsEnabled: Boolean, appPush: Boolean, serverPush: Boolean?): PushChip = when {
    !approvalDevice || !notificationsEnabled -> PushChip.Hidden
    !appPush || serverPush == false -> PushChip.OnlyWhileOpen
    else -> PushChip.On
}

/** The Integrations row's value: how many accounts are connected. */
internal fun accountsValue(n: Int): String = when (n) {
    0 -> "None"
    1 -> "1 account"
    else -> "$n accounts"
}

/** The Vault passkeys row's value: how many there are ([count] null while unknown). */
internal fun vaultPasskeysValue(count: Int?): String = when (count) {
    null -> "Not set up"
    0 -> "None"
    else -> "$count"
}

/** The Sounds & haptics row's value. */
internal fun soundsValue(s: FeedbackSettings): String = when {
    !s.master -> "Off"
    s.soundsOn && s.hapticsOn -> "On"
    s.soundsOn -> "Sounds only"
    s.hapticsOn -> "Haptics only"
    else -> "Off"
}

/** "claude.ai · 5 min ago" for an AI app, "computer · just now" for a computer, "never used" for either. */
internal fun connectionLine(connection: ConnectionView, nowSeconds: Long? = null): String {
    val used = connection.lastUsedAt?.let { if (nowSeconds != null) relativeTime(it, nowSeconds) else relativeTime(it) } ?: return "never used"
    val where = if (connection.keyFingerprint != null) "computer" else untrusted(connection.clientHost)
    return "$where · $used"
}

/** The server's host, as the account card shows it ("app.reins2fa.com"). */
internal fun hostOf(serverUrl: String): String =
    serverUrl.substringAfter("://").trimEnd('/')

/** A mode's icon in Settings and on an AI's page. */
internal fun settingsModeIcon(mode: AutopilotMode): ImageVector = when (mode) {
    AutopilotMode.MANUAL -> Icons.Rounded.HowToReg
    AutopilotMode.AUTO -> Icons.Rounded.Bolt
    AutopilotMode.OVERNIGHT -> dev.reins.android.ui.autopilot.OvernightIcon
    AutopilotMode.BYPASS -> Icons.Rounded.Bolt
    AutopilotMode.LOCKDOWN -> Icons.Rounded.Lock
}

internal fun copyText(context: Context, label: String, text: String) {
    context.getSystemService(ClipboardManager::class.java)?.setPrimaryClip(ClipData.newPlainText(label, text))
}
