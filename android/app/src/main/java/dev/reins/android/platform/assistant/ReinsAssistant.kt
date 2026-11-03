package dev.reins.android.platform.assistant

import android.app.PendingIntent
import androidx.appfunctions.AppFunctionAppUnknownException
import androidx.appfunctions.AppFunctionDeniedException
import androidx.appfunctions.AppFunctionElementNotFoundException
import androidx.appfunctions.AppFunctionException
import androidx.appfunctions.AppFunctionInvalidArgumentException
import androidx.appfunctions.AppFunctionPermissionRequiredException
import dev.reins.android.design.serviceName
import dev.reins.core.ActivityEntry
import dev.reins.core.ActivityQuery
import dev.reins.core.AutopilotMode
import dev.reins.core.AutopilotSettings
import dev.reins.core.ConnectionAutopilot
import dev.reins.core.CoreException
import dev.reins.core.GrantView
import dev.reins.core.PendingItem
import dev.reins.core.PendingKind
import dev.reins.core.ReinsCoreInterface
import java.time.Instant
import kotlin.coroutines.cancellation.CancellationException
import kotlinx.coroutines.CoroutineDispatcher
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.TimeoutCancellationException
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import kotlinx.coroutines.withContext
import kotlinx.coroutines.withTimeout

/**
 * What Gemini can do with Reins. It only takes power away: deny, lock down, stop a looser mode, revoke.
 * Approving stays in the app, through the [PendingIntent] from [openRequest].
 *
 * The service is a thin adapter over this. Tests pass a core and the callbacks the container would provide.
 */
class ReinsAssistant(
    private val core: ReinsCoreInterface,
    private val isLocked: () -> Boolean,
    /** The Settings switch. Off refuses every call, including ones that work while the phone is locked. */
    private val isAllowed: () -> Boolean,
    private val openScreen: (String) -> PendingIntent,
    private val afterDeny: (String?) -> Unit,
    private val playLockdown: () -> Unit,
    private val refreshTiles: () -> Unit,
    private val stopBypasses: suspend () -> Unit,
    private val stopOvernights: suspend () -> Unit,
    private val refreshAutopilot: suspend () -> Unit,
    private val dispatcher: CoroutineDispatcher = Dispatchers.IO,
    private val timeoutMs: Long = 10_000,
) {
    suspend fun getStatus(): ReinsStatus = call {
        requireSignedIn()
        val status = readStatus()
        guardAllowed()
        status
    }

    suspend fun listWaitingRequests(limit: Int?): List<WaitingRequest> = call {
        requireSignedIn()
        requireUnlocked()
        val n = clamp(limit, DEFAULT_WAITING, MAX_WAITING)
        val page = waitingRequests().sortedByDescending { it.createdAt }.take(n).map { it.toWaiting() }
        guardUnlocked()
        page
    }

    suspend fun openRequest(requestId: String): OpenedRequest = call {
        requireSignedIn()
        requireUnlocked()
        val item = waitingRequests().firstOrNull { it.id == requestId }
            ?: throw AppFunctionElementNotFoundException("That request is no longer waiting.")
        guardUnlocked()
        OpenedRequest(openScreen(item.id))
    }

    suspend fun denyRequest(requestId: String, note: String?): DeniedRequest = call {
        requireSignedIn()
        requireUnlocked()
        if (waitingRequests().none { it.id == requestId }) {
            throw AppFunctionElementNotFoundException("That request is no longer waiting.")
        }
        guardUnlocked()
        try {
            core.denyWithNote(requestId, note?.take(NOTE_MAX))
        } catch (e: CoreException.NotFound) {
            throw AppFunctionElementNotFoundException("That request is no longer waiting.")
        }
        afterDeny(requestId)
        refreshTiles()
        DeniedRequest(requestId)
    }

    suspend fun denyAllWaiting(): DeniedCount = call {
        requireSignedIn()
        requireUnlocked()
        var denied = 0
        var stopped = false
        for (item in waitingRequests()) {
            if (!isAllowed() || isLocked()) {
                stopped = true
                break
            }
            try {
                core.denyWithNote(item.id, null)
                denied += 1
            } catch (e: CoreException.NotFound) {
                // Denied a moment ago, from the phone or another call.
            }
        }
        if (!stopped || denied > 0) {
            afterDeny(null)
            refreshTiles()
        }
        DeniedCount(denied)
    }

    suspend fun startLockdown(): ReinsStatus = call {
        requireSignedIn()
        modeWrites.withLock {
            guardAllowed()
            // Re-read inside the lock so this cannot land between another mode write's check and its write.
            core.autopilotSettings()
            core.setAutopilotMode(null, AutopilotMode.LOCKDOWN, null)
        }
        playLockdown()
        refreshAutopilot()
        refreshTiles()
        val status = readStatus()
        guardAllowed()
        status
    }

    suspend fun switchToManual(): ReinsStatus = call {
        requireSignedIn()
        modeWrites.withLock {
            guardAllowed()
            // Lockdown is stricter than Manual. Stopping it here would loosen it, so the user ends it in Reins.
            refuseIfLockdown()
            stopBypasses()
            stopOvernights()
            refuseIfLockdown()
            val settings = core.autopilotSettings()
            for (connection in settings.connections) {
                if (!connection.looserThanManual()) continue
                guardAllowed()
                refuseIfLockdown()
                core.setAutopilotMode(connection.connectionId, AutopilotMode.MANUAL, null)
            }
            guardAllowed()
            refuseIfLockdown()
            core.setAutopilotMode(null, AutopilotMode.MANUAL, null)
        }
        refreshAutopilot()
        refreshTiles()
        val status = readStatus()
        guardAllowed()
        status
    }

    suspend fun searchActivity(ai: String?, integration: String?, since: Instant?, limit: Int?): List<ActivityItem> = call {
        requireSignedIn()
        requireUnlocked()
        val n = clamp(limit, DEFAULT_ACTIVITY, MAX_ACTIVITY)
        val entries = if (since != null) {
            // Empty text: the core must not search message bodies for words the agent typed.
            core.searchActivity(ActivityQuery(text = "", since = since.epochSecond), null, ACTIVITY_FETCH.toUInt()).hits.map { it.entry }
        } else {
            core.activity(ACTIVITY_FETCH.toUInt())
        }
        val aiFilter = ai?.trim()?.takeIf { it.isNotEmpty() }
        val integrationFilter = integration?.trim()?.takeIf { it.isNotEmpty() }
        val page = entries.filter { entry ->
            (aiFilter == null || entry.connectionLabel.contains(aiFilter, ignoreCase = true)) &&
                (integrationFilter == null || serviceName(entry.service).contains(integrationFilter, ignoreCase = true))
        }.sortedByDescending { it.at }.take(n).map { it.toActivity() }
        guardUnlocked()
        page
    }

    suspend fun listStandingPermissions(): List<StandingPermission> = call {
        requireSignedIn()
        requireUnlocked()
        val open = core.grants().filter { it.active }.sortedByDescending { it.createdAt }.map { it.toStanding() }
        guardUnlocked()
        open
    }

    suspend fun revokeStandingPermission(permissionId: String): RevokedPermission = call {
        requireSignedIn()
        requireUnlocked()
        if (core.grants().none { it.id == permissionId }) {
            throw AppFunctionElementNotFoundException("That permission is not open.")
        }
        guardUnlocked()
        try {
            core.revokeGrant(permissionId)
        } catch (e: CoreException.NotFound) {
            throw AppFunctionElementNotFoundException("That permission is not open.")
        }
        RevokedPermission(permissionId)
    }

    suspend fun listConnectedAis(): List<ConnectedAi> = call {
        requireSignedIn()
        requireUnlocked()
        val ais = core.connections().map { ConnectedAi(it.id, it.label, it.lastUsedAt?.let(Instant::ofEpochSecond)) }
        guardUnlocked()
        ais
    }

    /** Requests only. A pairing, an uploaded file or another phone is not something to deny by voice. */
    private suspend fun waitingRequests(): List<PendingItem> = core.pending().filter { it.kind == PendingKind.REQUEST }

    private suspend fun readStatus(): ReinsStatus {
        val settings = core.autopilotSettings()
        return ReinsStatus(
            mode = modeName(settings.mode),
            until = until(settings),
            waiting = waitingRequests().size,
            connectedAis = core.connections().size,
        )
    }

    private suspend fun requireSignedIn() {
        val session = try {
            core.session()
        } catch (e: CoreException.NotLoggedIn) {
            throw AppFunctionPermissionRequiredException("Sign in to Reins first")
        }
        if (session == null) throw AppFunctionPermissionRequiredException("Sign in to Reins first")
    }

    private fun requireUnlocked() {
        if (isLocked()) throw AppFunctionDeniedException("Unlock the phone first")
    }

    /** The switch can flip while a call is in flight. Reads and writes check again at the point of return or mutation. */
    private fun guardAllowed() {
        if (!isAllowed()) throw AppFunctionDeniedException("Gemini is off in Reins")
    }

    private fun guardUnlocked() {
        guardAllowed()
        requireUnlocked()
    }

    private suspend fun refuseIfLockdown() {
        if (core.autopilotSettings().mode == AutopilotMode.LOCKDOWN) {
            throw AppFunctionDeniedException("Lockdown is on; end it in Reins.")
        }
    }

    /** An agent's own mode that is looser than Manual. A connection in Lockdown is left alone. */
    private fun ConnectionAutopilot.looserThanManual(): Boolean {
        if (baseMode == AutopilotMode.LOCKDOWN) return false
        if (baseMode == AutopilotMode.AUTO || baseMode == AutopilotMode.BYPASS || baseMode == AutopilotMode.OVERNIGHT) return true
        return bypassUntil != null || overnightUntil != null
    }

    private suspend fun <T> call(block: suspend () -> T): T = try {
        // apply() can fail while the index is catching up, so a switch that is off must refuse here too.
        if (!isAllowed()) throw AppFunctionDeniedException("Gemini is off in Reins")
        withTimeout(timeoutMs) { withContext(dispatcher) { block() } }
    } catch (e: TimeoutCancellationException) {
        throw AppFunctionAppUnknownException("Reins did not answer in time")
    } catch (e: CancellationException) {
        throw e
    } catch (e: AppFunctionException) {
        throw e
    } catch (e: CoreException) {
        throw mapped(e)
    } catch (_: Exception) {
        throw AppFunctionAppUnknownException("Reins could not do that")
    }

    private fun mapped(error: CoreException): AppFunctionException = when (error) {
        is CoreException.NotLoggedIn -> AppFunctionPermissionRequiredException("Sign in to Reins first")
        is CoreException.NotFound -> AppFunctionElementNotFoundException("That is no longer there.")
        is CoreException.Invalid -> AppFunctionInvalidArgumentException(oneLine(error.reason, SUMMARY_MAX).ifBlank { "That cannot be done." })
        else -> AppFunctionAppUnknownException("Reins could not do that")
    }

    private fun PendingItem.toWaiting() = WaitingRequest(
        id = id,
        ai = connectionLabel,
        integration = serviceName(service),
        summary = waitingSummary(this),
        waitingSince = Instant.ofEpochSecond(createdAt),
        aiWaitsUntil = waitUntil?.let(Instant::ofEpochSecond),
        canApproveQuickly = quick,
    )

    private fun ActivityEntry.toActivity() = ActivityItem(
        at = Instant.ofEpochSecond(at),
        ai = connectionLabel,
        integration = serviceName(service),
        summary = activitySummary(this),
        outcome = outcomeName(outcome),
        decidedBy = deciderName(this),
    )

    private fun GrantView.toStanding() = StandingPermission(
        id = id,
        ai = connectionLabel,
        integration = serviceName(service),
        allows = grantAllows(this),
        expiresAt = expiresAt?.let(Instant::ofEpochSecond),
    )

    private companion object {
        const val DEFAULT_WAITING = 10
        const val MAX_WAITING = 25
        const val DEFAULT_ACTIVITY = 15
        const val MAX_ACTIVITY = 50
        const val ACTIVITY_FETCH = 200
        const val SUMMARY_MAX = 200
        const val NOTE_MAX = 500

        /** Every mode write in this process, so a lockdown cannot land between a check and a manual write. */
        val modeWrites = Mutex()

        /** Omitted and non-positive limits use [default]; anything past [max] is cut to it. */
        fun clamp(value: Int?, default: Int, max: Int): Int = when {
            value == null || value <= 0 -> default
            value > max -> max
            else -> value
        }

        fun modeName(mode: AutopilotMode): String = when (mode) {
            AutopilotMode.MANUAL -> "manual"
            AutopilotMode.AUTO -> "auto"
            AutopilotMode.BYPASS -> "bypass"
            AutopilotMode.OVERNIGHT -> "overnight"
            AutopilotMode.LOCKDOWN -> "lockdown"
        }

        /** A timed mode's end. `Long.MAX_VALUE` means "until you turn it off", which is no instant. */
        fun until(settings: AutopilotSettings): Instant? {
            val epoch = when (settings.mode) {
                AutopilotMode.BYPASS -> settings.bypassUntil
                AutopilotMode.OVERNIGHT -> settings.overnightUntil
                else -> null
            }
            return epoch?.takeIf { it in 1 until Long.MAX_VALUE }?.let(Instant::ofEpochSecond)
        }

        fun sensitive(service: String) = service == "vault" || service == "payments"

        fun waitingSummary(item: PendingItem): String = described(item.opTitle, item.action, item.service, item.count)

        fun activitySummary(entry: ActivityEntry): String = described(entry.opTitle, entry.action, entry.service, entry.count)

        /** Vault and payments stay the operation or the integration name. Nothing else is named. */
        fun grantAllows(grant: GrantView): String {
            val text = if (sensitive(grant.service)) {
                grant.action.ifBlank { serviceName(grant.service) }
            } else {
                generic(grant.action, grant.service)
            }
            return oneLine(text, SUMMARY_MAX)
        }

        /**
         * The operation title, or a phrase built from the action and the integration. A count above one is appended.
         * Headline, title, subject, label, detail and a grant's summary are never read.
         */
        fun described(opTitle: String, action: String, service: String, count: UInt): String {
            val base = if (sensitive(service)) {
                opTitle.trim().ifBlank { serviceName(service) }
            } else {
                opTitle.trim().ifBlank { generic(action, service) }
            }
            val shown = if (!sensitive(service) && count > 1u) "$base · $count" else base
            return oneLine(shown, SUMMARY_MAX)
        }

        fun generic(action: String, service: String): String {
            val verb = when (action) {
                "read" -> "Read"
                "write" -> "Write"
                "send" -> "Send"
                "search" -> "Search"
                "delete" -> "Delete"
                else -> action.trim().replaceFirstChar { if (it.isLowerCase()) it.titlecase() else it.toString() }
            }.ifBlank { "Use" }
            val name = serviceName(service)
            return if (name.isBlank()) verb else "$verb $name"
        }

        fun outcomeName(outcome: String): String = when (outcome) {
            "released", "sent", "granted" -> "approved"
            "denied" -> "denied"
            "error" -> "error"
            "revoked" -> "revoked"
            "held", "undone" -> outcome
            else -> outcome
        }

        fun deciderName(entry: ActivityEntry): String = when {
            entry.decidedBy.isEmpty() && entry.grantId != null -> "standing permission"
            entry.decidedBy.isEmpty() -> "you"
            entry.decidedBy == "bypass" -> "bypass"
            entry.decidedBy == "lockdown" -> "lockdown"
            else -> "autopilot"
        }

        fun oneLine(text: String, max: Int): String = text.replace(Regex("\\s+"), " ").trim().take(max)
    }
}
