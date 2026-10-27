package dev.reins.android.ui.settings

import dev.reins.android.design.pane
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

import dev.reins.android.design.ButtonSize
import dev.reins.android.design.ButtonStyle
import dev.reins.android.design.Glyph
import dev.reins.android.design.GlyphIcon
import dev.reins.android.design.Pill
import dev.reins.android.design.SignalButton
import dev.reins.android.design.SignalDialog
import dev.reins.android.design.hairlineBelow
import dev.reins.android.design.Appearance
import dev.reins.android.ui.main.RootHeader
import androidx.compose.foundation.layout.defaultMinSize
import androidx.compose.foundation.verticalScroll

/**
 * Settings (Reins Signal, iOS-Settings 01 / And-Core 05): only you and this phone. The account, then Devices, Sounds
 * and vibration, Notification channels, Appearance, the recovery code, vault passkeys and the tour as monochrome rows;
 * below the fold the version and updates, signing out and deleting the account, Terms and Privacy. Connections and
 * Autopilot have their own tabs now.
 */
@Composable
fun SettingsScreen(
    state: AppState,
    viewModel: SettingsViewModel,
    onSounds: () -> Unit,
    onAppearance: () -> Unit = {},
    onWallpaper: () -> Unit = {},
    onTour: () -> Unit = {},
    onVaultPasskeys: () -> Unit = {},
    onDevices: () -> Unit = {},
    /** Profiles > Add work account. */
    onAddWorkAccount: () -> Unit = {},
    /** Debug builds: the Signal component gallery. */
    onSignalGallery: () -> Unit = {},
    authenticator: Authenticator,
) {
    val c = LocalColors.current
    val feedback = LocalFeedback.current
    val context = LocalContext.current
    val ui by viewModel.ui.collectAsStateWithLifecycle()
    val sounds by viewModel.sounds.collectAsStateWithLifecycle()
    val session by state.session.collectAsStateWithLifecycle()
    val hasRecoveryCode by viewModel.hasRecoveryCode.collectAsStateWithLifecycle()
    val recoveryCode by viewModel.recoveryCode.collectAsStateWithLifecycle()
    val deletion by viewModel.deletion.collectAsStateWithLifecycle()
    val vaultPasskeys by viewModel.vaultPasskeys.collectAsStateWithLifecycle()
    val devices by viewModel.devices.collectAsStateWithLifecycle()
    val appearance by Appearance.state(context)
    var confirmSignOut by remember { mutableStateOf(false) }
    LaunchedEffect(session) {
        viewModel.checkRecoveryCode()
        viewModel.countDevices()
    }
    val signedIn = session as? SessionState.SignedIn
    val channels = remember { notificationChannelCount(context) }

    // Next (Settings 01): the account on its card, then this phone, security and Reins, each a raised group.
    Box(Modifier.fillMaxSize().background(c.paper)) {
        Column(Modifier.fillMaxSize().verticalScroll(androidx.compose.foundation.rememberScrollState())) {
            RootHeader("Settings")
            androidx.compose.runtime.CompositionLocalProvider(LocalRaisedGroups provides true) {
                signedIn?.let {
                    Box(Modifier.padding(horizontal = 16.dp).padding(top = 14.dp).fillMaxWidth().pane(radius = 24.dp).padding(14.dp)) {
                        AccountBlock(it.info.email, it.info.serverUrl, state, viewModel)
                    }
                }
                if (signedIn != null) {
                    val app = androidx.compose.ui.platform.LocalContext.current.applicationContext as? dev.reins.android.ReinsApp
                    app?.let { ProfilesGroup(it.container, onAddWorkAccount) }
                }
                ui.message?.let { Banner(it, Modifier.padding(horizontal = 16.dp).padding(top = 10.dp)) }
                ui.error?.let { Banner(it, Modifier.padding(horizontal = 16.dp).padding(top = 10.dp), BannerKind.Error) }

                SettingsGroup(header = "This phone", glyph = Glyph.Phone) {
                    if (signedIn != null) {
                        SetRow("Devices", Glyph.Devices, Modifier.testTag("devicesRow"), value = devices?.toString(), onClick = onDevices)
                    }
                    SetRow("Sounds", Glyph.Speaker, Modifier.testTag("openSounds"), value = soundsValue(sounds), onClick = onSounds)
                    SetRow("Notification channels", Glyph.Bell, Modifier.testTag("notificationChannels"), value = channels.takeIf { it > 0 }?.toString()) {
                        openNotificationChannels(context)
                    }
                    SetRow("Appearance", Glyph.Appearance, Modifier.testTag("openAppearance"), value = appearance.theme.label, onClick = onAppearance)
                    val dark = c.dark
                    val wall = appearance.wallpaper(dark)
                    SetRow(
                        "Wallpaper",
                        Glyph.Photo,
                        Modifier.testTag("openWallpaper"),
                        value = wall.image.label,
                        divider = false,
                        leading = { WallpaperThumb(wall, dark, Modifier.size(32.dp), radius = 9.dp) },
                        onClick = onWallpaper,
                    )
                }
                if (signedIn != null && hasRecoveryCode) {
                    SettingsGroup(header = "Security", glyph = Glyph.Shield) {
                        SetRow("Recovery code", Glyph.Lifebuoy, Modifier.testTag("recoveryCodeRow")) { viewModel.showRecoveryCode(authenticator) }
                        SetRow("Passkeys", Glyph.Fingerprint, Modifier.testTag("vaultPasskeysRow"), value = vaultPasskeysValue(vaultPasskeys), divider = false, onClick = onVaultPasskeys)
                    }
                }
                SettingsGroup(header = "Reins", glyph = Glyph.Help) {
                    SetRow("Tour", Glyph.Play, Modifier.testTag("takeTour"), onClick = onTour)
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
                    SetRow("Sign out", Glyph.Quit, Modifier.testTag("signOut"), destructive = true, chevron = false) { if (!ui.busy) confirmSignOut = true }
                    if (signedIn != null) {
                        SetRow("Delete account", Glyph.Trash, Modifier.testTag("deleteAccount"), destructive = true, chevron = false, divider = false, onClick = viewModel::openDeletion)
                    }
                }

                Footer()

                if (BuildConfig.DEBUG) {
                    // Debug builds only: every component, to compare with the boards on a phone.
                    SettingsGroup(header = "Developer") {
                        SetRow("Component gallery", Glyph.Apps, Modifier.testTag("signalGalleryRow"), divider = false, onClick = onSignalGallery)
                    }
                }
            }
            Spacer(Modifier.height(32.dp).testTag("pageEnd"))
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
        SignalDialog(
            title = "Sign out?",
            text = "Requests can't reach this phone until you sign in again.",
            confirmLabel = "Sign out",
            destructive = true,
            onConfirm = {
                confirmSignOut = false
                viewModel.signOut()
            },
            onDismiss = { confirmSignOut = false },
        )
    }
}

/**
 * A row of Settings (Next `.r`): a monochrome 32 dp tile (or [leading]), the title, a value in ink-2, a chevron.
 * [destructive] draws the title and glyph in danger. Its hairline is below it; the group clips the last one.
 */
@Composable
fun SetRow(
    title: String,
    glyph: Glyph,
    modifier: Modifier = Modifier,
    value: String? = null,
    destructive: Boolean = false,
    chevron: Boolean = true,
    divider: Boolean = true,
    leading: (@Composable () -> Unit)? = null,
    onClick: () -> Unit,
) {
    val c = LocalColors.current
    Row(
        modifier
            .fillMaxWidth()
            .defaultMinSize(minHeight = 56.dp)
            .hairlineBelow(c.rule, inset = 44.dp)
            .pressable(highlight = c.paperSunken, shape = RoundedCornerShape(0.dp), onClick = onClick)
            .padding(vertical = 9.dp),
        horizontalArrangement = Arrangement.spacedBy(12.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        if (leading != null) {
            leading()
        } else {
            Box(
                Modifier.size(32.dp).background(if (destructive) c.dangerWash else c.paperSunken, RoundedCornerShape(9.dp)),
                contentAlignment = Alignment.Center,
            ) { GlyphIcon(glyph, if (destructive) c.danger else c.ink2, size = 18.dp) }
        }
        RText(title, RType.row, if (destructive) c.danger else c.ink, Modifier.weight(1f), maxLines = 1)
        value?.let { RText(it, RType.sans(15f), c.ink2, maxLines = 1) }
        if (chevron) GlyphIcon(Glyph.ChevronRight, c.inkFaint, size = 18.dp)
    }
}

/** How many notification channels Reins has (the row opens Android's page for them). */
internal fun notificationChannelCount(context: Context): Int =
    androidx.core.app.NotificationManagerCompat.from(context).notificationChannelsCompat.size

/** Android's notification page for Reins, where each channel can be changed. */
internal fun openNotificationChannels(context: Context) {
    val intent = android.content.Intent(android.provider.Settings.ACTION_APP_NOTIFICATION_SETTINGS)
        .putExtra(android.provider.Settings.EXTRA_APP_PACKAGE, context.packageName)
        .addFlags(android.content.Intent.FLAG_ACTIVITY_NEW_TASK)
    try {
        context.startActivity(intent)
    } catch (_: android.content.ActivityNotFoundException) {
    }
}

/**
 * Who is signed in and to which server (a tap copies it), then whether this phone approves and is notified: green
 * pills when all is well, or the one thing to do about it.
 */
@OptIn(ExperimentalLayoutApi::class)
@Composable
private fun AccountBlock(email: String, serverUrl: String, state: AppState, viewModel: SettingsViewModel) {
    val c = LocalColors.current
    val feedback = LocalFeedback.current
    val context = LocalContext.current
    val ui by viewModel.ui.collectAsStateWithLifecycle()
    val approvalDevice by state.approvalDevice.collectAsStateWithLifecycle()
    val serverPush by viewModel.serverPush.collectAsStateWithLifecycle()
    val notifications = rememberNotificationState()
    val push = pushChip(approvalDevice, notifications.enabled, appPushOverride ?: FirebaseSupport.available(context), serverPush)
    Column(Modifier.fillMaxWidth().hairlineBelow(c.rule).padding(bottom = 14.dp).testTag("accountCard")) {
        Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(14.dp)) {
            Box(Modifier.size(54.dp).background(c.paperSunken, CircleShape), contentAlignment = Alignment.Center) {
                RText(email.take(1).uppercase(), RType.serif(28f, 30f), c.ink, maxLines = 1)
            }
            Column(Modifier.weight(1f)) {
                RText(email, RType.sans(17f, FontWeight.SemiBold), c.ink, Modifier.testTag("accountEmail"), maxLines = 1, ltr = true)
                RText(
                    hostOf(serverUrl),
                    RType.mono(12.5f),
                    c.inkMeta,
                    Modifier
                        .pressable(shape = RoundedCornerShape(6.dp), label = "Copy server address") {
                            copyText(context, "Reins server", serverUrl)
                            feedback.play(Event.Copied)
                            // Android 13 and later confirm a copy themselves.
                            if (Build.VERSION.SDK_INT < Build.VERSION_CODES.TIRAMISU) viewModel.notice("Server address copied.")
                        }
                        .testTag("copyServer"),
                    maxLines = 1,
                    ltr = true,
                )
                FlowRow(Modifier.padding(top = 6.dp), horizontalArrangement = Arrangement.spacedBy(6.dp), verticalArrangement = Arrangement.spacedBy(6.dp)) {
                    if (approvalDevice) {
                        Pill("Approval phone", c.success, Modifier.testTag("approvalChip"), glyph = Glyph.Check)
                    } else {
                        Pill("Another phone approves", c.warning, Modifier.testTag("approvalChip"))
                    }
                    when (push) {
                        PushChip.On -> Pill("Notifications on", c.success, Modifier.testTag("notificationsChip"))
                        PushChip.OnlyWhileOpen -> Pill("Only while open", c.inkMeta, Modifier.testTag("notificationsChip"))
                        PushChip.Hidden -> Unit
                    }
                }
            }
        }
        if (!notifications.enabled) {
            Row(
                Modifier.padding(top = 14.dp).fillMaxWidth().testTag("notificationsRow"),
                horizontalArrangement = Arrangement.spacedBy(10.dp),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                GlyphIcon(Glyph.Warning, c.warning, size = 18.dp)
                RText("Notifications are off", RType.sans(14.5f, FontWeight.Medium), c.ink, Modifier.weight(1f), maxLines = 2)
                SignalButton("Turn on", Modifier.testTag("turnOnNotifications"), style = ButtonStyle.Soft, size = ButtonSize.Compact) { notifications.request() }
            }
        }
        if (!approvalDevice) {
            SignalButton(
                "Use this phone for approvals",
                Modifier.padding(top = 14.dp).fillMaxWidth().testTag("registerPhone"),
                size = ButtonSize.Small,
                glyph = Glyph.Phone,
                enabled = !ui.busy,
                onClick = viewModel::registerThisPhone,
            )
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
