package dev.reins.android.ui.settings

import android.app.Activity
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.browser.auth.AuthTabIntent
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.unit.dp
import androidx.lifecycle.ViewModel
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.lifecycle.viewModelScope
import dev.reins.android.AppContainer
import dev.reins.android.design.Banner
import dev.reins.android.design.BannerKind
import dev.reins.android.design.ButtonStyle
import dev.reins.android.design.Glyph
import dev.reins.android.design.ProfileBadge
import dev.reins.android.design.RTextField
import dev.reins.android.design.SignalButton
import dev.reins.android.platform.Browser
import dev.reins.android.platform.SsoPurpose
import dev.reins.android.profiles.ProfileSwitch
import dev.reins.android.profiles.ProfilesProvider
import dev.reins.android.profiles.WorkProfiles
import dev.reins.android.ui.common.untrustedName
import dev.reins.android.ui.common.userMessage
import dev.reins.android.ui.signin.AccountRules
import dev.reins.core.AccountProfile
import dev.reins.core.CoreException
import dev.reins.core.FamilyView
import dev.reins.core.ReinsCoreInterface
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.filterNotNull
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

/** Where "Add family account" is. */
sealed interface AddFamilyStep {
    data object Link : AddFamilyStep
    data object Working : AddFamilyStep

    /** Joined: switch to it. */
    data class Added(val profile: AccountProfile) : AddFamilyStep
}

data class AddFamilyUi(val step: AddFamilyStep = AddFamilyStep.Link, val error: String? = null)

/**
 * "Add family account" (P7, 08 R6, R7): the invitation link a guardian sent, then a sign-in with a **new** account on
 * its own reserved profile and core, which joins the family. The Personal account never takes part; an account that
 * isn't new is refused by the server, and the reserved profile removed.
 */
class AddFamilyAccountViewModel(private val container: AppContainer) : ViewModel() {
    private val _ui = MutableStateFlow(AddFamilyUi())
    val ui: StateFlow<AddFamilyUi> = _ui.asStateFlow()

    val registry get() = container.profiles

    private var profile: AccountProfile? = null
    private var family: ReinsCoreInterface? = null
    private var link: String = ""

    init {
        viewModelScope.launch {
            container.ssoSignIn.callback.filterNotNull().collect { finish() }
        }
    }

    private fun server(): String = container.state.session.value.let { it as? dev.reins.android.state.SessionState.SignedIn }
        ?.info?.serverUrl ?: dev.reins.android.BuildConfig.DEFAULT_SERVER

    fun start(invite: String, open: (String) -> Unit) {
        if (_ui.value.step == AddFamilyStep.Working) return
        if (!invite.trim().startsWith("reins-family://") && invite.trim().length != TOKEN_LENGTH) {
            _ui.value = AddFamilyUi(error = "This isn't a family invitation.")
            return
        }
        link = invite.trim()
        _ui.value = AddFamilyUi(AddFamilyStep.Working)
        viewModelScope.launch {
            try {
                val server = server()
                val reserved = withContext(Dispatchers.IO) { container.profiles.beginManaged(server) }
                profile = reserved
                val core = ProfilesProvider.workCore.create(container.appContext, reserved)
                family = core
                val start = core.ssoBegin(server)
                container.ssoSignIn.begin(server, start, SsoPurpose.AddFamily)
                open(start.url)
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                drop()
                _ui.value = AddFamilyUi(error = e.userMessage())
            }
        }
    }

    private suspend fun finish() {
        val (pending, url) = container.ssoSignIn.take(SsoPurpose.AddFamily) ?: return
        val reserved = profile ?: return
        val core = family ?: return
        _ui.value = AddFamilyUi(AddFamilyStep.Working)
        try {
            val outcome = core.ssoFinish(pending.server, url, pending.state, pending.verifier)
            val joined = core.familyJoin(link)
            val added = withContext(Dispatchers.IO) {
                container.profiles.confirmFamily(reserved.id, joined.orgId, joined.name, outcome.session.email)
            }
            profile = null
            runCatching { core.pushTag() }.getOrNull()?.let { dev.reins.android.push.PushRouting.remember(container.appContext, added.id, it) }
            runCatching { core.registerDevice(null) }
            _ui.value = AddFamilyUi(AddFamilyStep.Added(added))
        } catch (e: CancellationException) {
            throw e
        } catch (e: CoreException) {
            drop()
            _ui.value = AddFamilyUi(error = e.userMessage())
        }
    }

    private suspend fun drop() {
        val id = profile?.id ?: return
        profile = null
        family = null
        withContext(Dispatchers.IO) { runCatching { container.profiles.remove(id) } }
    }

    private companion object {
        const val TOKEN_LENGTH = 43
    }
}

@Composable
fun AddFamilyAccountScreen(viewModel: AddFamilyAccountViewModel, onBack: () -> Unit) {
    val context = LocalContext.current
    val ui by viewModel.ui.collectAsStateWithLifecycle()
    var invite by remember { mutableStateOf("") }
    val authTab = rememberLauncherForActivityResult(AuthTabIntent.AuthenticateUserResultContract()) { result ->
        if (result.resultCode == AuthTabIntent.RESULT_OK) {
            result.resultUri?.let { (context.applicationContext as dev.reins.android.ReinsApp).container.ssoSignIn.deliver(it.toString()) }
        }
    }
    SettingsPage("Add family account", onBack) {
        Column(Modifier.fillMaxWidth().padding(16.dp), verticalArrangement = Arrangement.spacedBy(12.dp)) {
            ui.error?.let { Banner(it, kind = BannerKind.Error, tag = "addFamilyError") }
            when (val step = ui.step) {
                AddFamilyStep.Link, AddFamilyStep.Working -> {
                    RTextField(invite, { invite = it }, "Invitation link", tag = "familyInvite", enabled = step == AddFamilyStep.Link)
                    SignalButton("Join with a new account", Modifier.fillMaxWidth().testTag("addFamilyContinue"), glyph = Glyph.Family) {
                        viewModel.start(invite) { Browser.openSignIn(context, it, AccountRules.SSO_CALLBACK_SCHEME, authTab) }
                    }
                }
                is AddFamilyStep.Added -> {
                    ProfileBadge(active = WorkProfiles.of(step.profile))
                    SignalButton("Switch to ${WorkProfiles.label(step.profile)}", Modifier.fillMaxWidth().testTag("switchToFamily"), glyph = Glyph.Family) {
                        (context as? Activity)?.let { ProfileSwitch.switchTo(it, viewModel.registry, step.profile.id) }
                    }
                }
            }
        }
    }
}

data class FamilyUi(val view: FamilyView? = null, val error: String? = null, val busy: Boolean = false)

/** A family profile's own page: what the family sees, leaving, a guardian's question, and (guardians) the asks. */
class FamilyViewModel(private val core: ReinsCoreInterface) : ViewModel() {
    private val _ui = MutableStateFlow(FamilyUi())
    val ui: StateFlow<FamilyUi> = _ui.asStateFlow()

    fun load() = act { }

    fun leave() = act { core.familyRequestLeave() }

    fun stay() = act { core.familyCancelLeave() }

    fun allow(member: String) = act { core.familyConsentLeave(member) }

    fun answerMinor(accept: Boolean) = act { core.familyAnswerMinor(accept) }

    private fun act(step: suspend () -> Unit) {
        viewModelScope.launch {
            _ui.value = _ui.value.copy(busy = true, error = null)
            try {
                step()
                _ui.value = FamilyUi(core.familyView())
            } catch (e: CancellationException) {
                throw e
            } catch (e: CoreException) {
                _ui.value = _ui.value.copy(busy = false, error = e.userMessage())
            }
        }
    }
}

/** "Leaves in 3 days" style: whole days left, at least 1 while it hasn't happened. */
fun leavesIn(effectiveAt: Long, now: Long = System.currentTimeMillis() / 1000): String {
    val days = ((effectiveAt - now + 86_399) / 86_400).coerceAtLeast(0)
    return when (days) {
        0L -> "Leaving now"
        1L -> "Leaves in 1 day"
        else -> "Leaves in $days days"
    }
}

@Composable
fun FamilyScreen(viewModel: FamilyViewModel, onBack: () -> Unit) {
    val ui by viewModel.ui.collectAsStateWithLifecycle()
    LaunchedEffect(viewModel) { viewModel.load() }
    val v = ui.view
    SettingsPage(v?.name?.let { untrustedName(it, 40) } ?: "Family", onBack) {
        Column(Modifier.fillMaxWidth().padding(16.dp), verticalArrangement = Arrangement.spacedBy(12.dp)) {
            ui.error?.let { Banner(it, kind = BannerKind.Error, tag = "familyError") }
            if (v == null) return@Column
            ProfileBadge(active = WorkProfiles.active.copy(family = true))
            if (v.minorProposed) {
                Banner("Mark this account a minor's?", tag = "minorAsk")
                Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                    SignalButton("Accept", Modifier.weight(1f).testTag("minorAccept"), enabled = !ui.busy) { viewModel.answerMinor(true) }
                    SignalButton("Decline", Modifier.weight(1f).testTag("minorDecline"), style = ButtonStyle.Outline, enabled = !ui.busy) {
                        viewModel.answerMinor(false)
                    }
                }
            }
            SettingsGroup(header = "What your family sees") {
                v.sees.forEachIndexed { i, line ->
                    SetRow(line, Glyph.Eye, Modifier.testTag("sees:$i"), chevron = false, divider = i < v.sees.size - 1) {}
                }
            }
            if (v.guardian) {
                v.leaves.forEach { ask ->
                    val who = untrustedName(ask.email)
                    if (ask.effectiveAt == null) {
                        Banner("$who asked to leave. Allow?", tag = "leaveAsk:${ask.member}")
                        SignalButton("Allow", Modifier.fillMaxWidth().testTag("allowLeave:${ask.member}"), glyph = Glyph.Check, enabled = !ui.busy) {
                            viewModel.allow(ask.member)
                        }
                    } else {
                        Banner("$who · ${leavesIn(ask.effectiveAt!!)}", tag = "leaving:${ask.member}")
                    }
                }
            } else {
                val leave = v.leave
                when {
                    leave == null -> SignalButton(
                        "Leave",
                        Modifier.fillMaxWidth().testTag("familyLeave"),
                        style = ButtonStyle.Outline,
                        trailing = if (v.minor) null else "${v.noticeDays} days' notice",
                        enabled = !ui.busy,
                    ) { viewModel.leave() }
                    leave.needsConsent -> Banner("Waiting for a guardian", tag = "leaveWaiting")
                    else -> Banner(leavesIn(leave.effectiveAt ?: 0), tag = "leaveCountdown")
                }
                if (leave != null) {
                    SignalButton("Stay", Modifier.fillMaxWidth().testTag("familyStay"), style = ButtonStyle.Outline, enabled = !ui.busy) {
                        viewModel.stay()
                    }
                }
            }
        }
    }
}
