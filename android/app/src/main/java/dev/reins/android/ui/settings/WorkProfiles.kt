package dev.reins.android.ui.settings

import android.app.Activity
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.browser.auth.AuthTabIntent
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.produceState
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
import dev.reins.android.design.Glyph
import dev.reins.android.design.RTextField
import dev.reins.android.design.SignalButton
import dev.reins.android.platform.Browser
import dev.reins.android.platform.SsoPurpose
import dev.reins.android.profiles.Posture
import dev.reins.android.profiles.ProfileSwitch
import dev.reins.android.profiles.ProfilesProvider
import dev.reins.android.profiles.WorkProfiles
import dev.reins.android.profiles.enrollment
import dev.reins.android.ui.common.userMessage
import dev.reins.android.ui.signin.AccountRules
import dev.reins.core.AccountProfile
import dev.reins.core.CoreException
import dev.reins.core.ProfileKind
import dev.reins.core.ReinsCoreInterface
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.filterNotNull
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

/**
 * Settings → Profiles: Personal and each work profile, the active one marked; tapping another switches to it. Shown
 * only where the server has organizations, or the phone already has a work profile.
 */
@Composable
fun ProfilesGroup(container: AppContainer, server: String, onAddWork: () -> Unit) {
    val activity = LocalContext.current as? Activity
    val profiles by produceState(emptyList<AccountProfile>(), container) {
        value = withContext(Dispatchers.IO) { runCatching { container.profiles.list() }.getOrDefault(emptyList()) }
    }
    val offered by produceState(false, server) {
        value = runCatching { container.core.orgsOffered(server) }.getOrDefault(false)
    }
    if (!offered && profiles.none { it.kind == ProfileKind.MANAGED }) return
    val active = WorkProfiles.active.id
    SettingsGroup(header = "Profiles", glyph = Glyph.Person) {
        profiles.forEach { p ->
            val managed = p.kind == ProfileKind.MANAGED
            SetRow(
                WorkProfiles.label(p),
                if (managed) Glyph.Briefcase else Glyph.Person,
                Modifier.testTag("profile:${p.id}"),
                value = if (p.id == active) "In use" else p.email?.let { dev.reins.android.ui.common.untrustedName(it) },
                chevron = p.id != active,
            ) {
                if (p.id != active && activity != null) ProfileSwitch.switchTo(activity, container.profiles, p.id)
            }
        }
        SetRow("Add work account", Glyph.Briefcase, Modifier.testTag("addWorkAccount"), onClick = onAddWork)
    }
}

/** Where "Add work account" is. */
sealed interface AddWorkStep {
    data object Domain : AddWorkStep
    data object Working : AddWorkStep

    /** Added: switch to it. */
    data class Added(val profile: AccountProfile, val enrolled: Boolean) : AddWorkStep
}

data class AddWorkUi(val step: AddWorkStep = AddWorkStep.Domain, val error: String? = null)

/**
 * "Add work account" (05 §9, 06 §2): the work account signs in on its own core, in its own reserved profile, through
 * its organization's SSO (the domain hint); an account that isn't an org's member is refused and its profile removed.
 * Then this phone is enrolled to the org: a key in StrongBox or the TEE, attested, and the posture.
 */
class AddWorkAccountViewModel(private val container: AppContainer) : ViewModel() {
    private val _ui = MutableStateFlow(AddWorkUi())
    val ui: StateFlow<AddWorkUi> = _ui.asStateFlow()

    /** The phone's profiles (switching to the new one). */
    val registry get() = container.profiles

    private var profile: AccountProfile? = null
    private var work: ReinsCoreInterface? = null

    init {
        viewModelScope.launch {
            container.ssoSignIn.callback.filterNotNull().collect { finish() }
        }
    }

    private fun server(): String = container.state.session.value.let { it as? dev.reins.android.state.SessionState.SignedIn }
        ?.info?.serverUrl ?: dev.reins.android.BuildConfig.DEFAULT_SERVER

    fun start(domain: String, open: (String) -> Unit) {
        if (_ui.value.step == AddWorkStep.Working) return
        _ui.value = AddWorkUi(AddWorkStep.Working)
        viewModelScope.launch {
            try {
                val server = server()
                val reserved = withContext(Dispatchers.IO) { container.profiles.beginManaged(server) }
                profile = reserved
                val core = ProfilesProvider.workCore.create(container.appContext, reserved)
                work = core
                val start = core.ssoBeginWork(server, domain.trim())
                container.ssoSignIn.begin(server, start, SsoPurpose.AddWork)
                open(start.url)
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                drop()
                _ui.value = AddWorkUi(error = e.userMessage())
            }
        }
    }

    private suspend fun finish() {
        val (pending, url) = container.ssoSignIn.take(SsoPurpose.AddWork) ?: return
        val reserved = profile ?: return
        val core = work ?: return
        _ui.value = AddWorkUi(AddWorkStep.Working)
        try {
            val outcome = core.ssoFinish(pending.server, url, pending.state, pending.verifier)
            val org = core.orgMembership()
            if (org == null) {
                drop()
                _ui.value = AddWorkUi(error = "This account isn't managed by an organization.")
                return
            }
            val added = withContext(Dispatchers.IO) {
                container.profiles.confirmManaged(reserved.id, org.orgId, org.name, outcome.session.email)
            }
            profile = null
            val enrolled = runCatching {
                core.registerDevice(null)
                val challenge = core.orgChallenge()
                val input = withContext(Dispatchers.IO) {
                    enrollment(ProfilesProvider.deviceKeys, added.id, challenge, Posture.collect(container.appContext))
                }
                core.orgEnroll(challenge, input)
            }.isSuccess
            _ui.value = AddWorkUi(AddWorkStep.Added(added, enrolled))
        } catch (e: CancellationException) {
            throw e
        } catch (e: CoreException) {
            drop()
            _ui.value = AddWorkUi(error = e.userMessage())
        }
    }

    /** Forgets an unfinished work profile and what it kept. */
    private suspend fun drop() {
        val id = profile?.id ?: return
        profile = null
        work = null
        withContext(Dispatchers.IO) { runCatching { container.profiles.remove(id) } }
    }
}

@Composable
fun AddWorkAccountScreen(viewModel: AddWorkAccountViewModel, onBack: () -> Unit) {
    val context = LocalContext.current
    val ui by viewModel.ui.collectAsStateWithLifecycle()
    var domain by remember { mutableStateOf("") }
    val authTab = rememberLauncherForActivityResult(AuthTabIntent.AuthenticateUserResultContract()) { result ->
        if (result.resultCode == AuthTabIntent.RESULT_OK) {
            result.resultUri?.let { (context.applicationContext as dev.reins.android.ReinsApp).container.ssoSignIn.deliver(it.toString()) }
        }
    }
    SettingsPage("Add work account", onBack) {
        Column(Modifier.fillMaxWidth().padding(16.dp), verticalArrangement = Arrangement.spacedBy(12.dp)) {
            ui.error?.let { Banner(it, kind = BannerKind.Error, tag = "addWorkError") }
            when (val step = ui.step) {
                AddWorkStep.Domain, AddWorkStep.Working -> {
                    RTextField(domain, { domain = it }, "Work domain, like acme.com", tag = "workDomain", enabled = step == AddWorkStep.Domain)
                    SignalButton(
                        "Continue",
                        Modifier.fillMaxWidth().testTag("addWorkContinue"),
                        glyph = Glyph.Briefcase,
                    ) { viewModel.start(domain) { Browser.openSignIn(context, it, AccountRules.SSO_CALLBACK_SCHEME, authTab) } }
                }
                is AddWorkStep.Added -> {
                    val label = WorkProfiles.label(step.profile)
                    Banner(if (step.enrolled) "$label added. This phone is enrolled." else "$label added.")
                    SignalButton("Switch to $label", Modifier.fillMaxWidth().testTag("switchToWork"), glyph = Glyph.Briefcase) {
                        (context as? Activity)?.let { ProfileSwitch.switchTo(it, viewModel.registry, step.profile.id) }
                    }
                }
            }
        }
    }
}
