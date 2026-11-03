@file:OptIn(androidx.appfunctions.ExperimentalAppFunctionsApi::class)

package dev.reins.android.platform.assistant

import android.app.KeyguardManager
import android.content.Context
import android.os.Build
import androidx.annotation.RequiresApi
import androidx.appfunctions.AppFunctionAccessLevel
import androidx.appfunctions.AppFunctionDeclaration
import androidx.appfunctions.AppFunctionService
import androidx.appfunctions.AppFunctionServiceEntryPoint
import androidx.appfunctions.metadata.AppFunctionMetadata
import dev.reins.android.ReinsApp
import dev.reins.android.feedback.Event
import dev.reins.android.feedback.play
import dev.reins.android.platform.AppNotifier
import dev.reins.android.platform.tiles.Tiles
import java.time.Instant

/**
 * Gemini's entry to Reins. The functions are off until the user turns Gemini on in Settings.
 * Each one delegates to [ReinsAssistant]; this class only reaches this process's container.
 */
@RequiresApi(Build.VERSION_CODES.BAKLAVA)
@AppFunctionServiceEntryPoint(
    serviceName = "ReinsAppFunctionService",
    appFunctionXmlFileName = "reins_app_functions",
)
@AppFunctionAccessLevel(
    level = AppFunctionMetadata.ACCESS_LEVEL_ANDROID_TRUSTED,
    isCompatEnforcementEnabled = true,
)
abstract class ReinsAppFunctions : AppFunctionService() {
    private val assistant: ReinsAssistant by lazy {
        val container = (application as ReinsApp).container
        ReinsAssistant(
            core = container.core,
            isLocked = { deviceLocked(this) },
            openScreen = { id -> AppNotifier.openRequest(this, id, container.profile) },
            afterDeny = { id -> container.refreshAfterAnswer(id) },
            playLockdown = { container.feedback.play(Event.LockdownOn) },
            refreshTiles = { Tiles.refresh(this) },
            stopBypasses = { container.stopBypasses() },
            stopOvernights = { container.stopOvernights() },
            refreshAutopilot = { container.refreshAutopilot(); Unit },
            isAllowed = { AssistantStore(this).enabled((application as ReinsApp).container.profile.id) },
        )
    }

    /**
     * Reports the Reins mode, when a timed mode ends, how many requests are waiting, and how many AI agents are connected.
     *
     * Answers while the phone is locked.
     */
    @AppFunctionDeclaration(isDescribedByKDoc = true, isEnabled = false)
    @AppFunctionAccessLevel(level = AppFunctionMetadata.ACCESS_LEVEL_ANDROID_TRUSTED, isCompatEnforcementEnabled = true)
    suspend fun getStatus(): ReinsStatus = assistant.getStatus()

    /**
     * Lists requests waiting for the user, newest first.
     *
     * Does not include pairing requests or another phone asking to join the account. The phone must be unlocked.
     *
     * @param limit How many to return. 10 when omitted or not positive. Never more than 25.
     */
    @AppFunctionDeclaration(isDescribedByKDoc = true, isEnabled = false)
    @AppFunctionAccessLevel(level = AppFunctionMetadata.ACCESS_LEVEL_ANDROID_TRUSTED, isCompatEnforcementEnabled = true)
    suspend fun listWaitingRequests(limit: Int? = null): List<WaitingRequest> = assistant.listWaitingRequests(limit)

    /**
     * Opens one waiting request in Reins so the user can approve or deny it there.
     *
     * Returns a screen to open. This does not approve the request. The phone must be unlocked.
     *
     * @param requestId The id from listWaitingRequests.
     * @return The screen that opens that request.
     */
    @AppFunctionDeclaration(isDescribedByKDoc = true, isEnabled = false)
    @AppFunctionAccessLevel(level = AppFunctionMetadata.ACCESS_LEVEL_ANDROID_TRUSTED, isCompatEnforcementEnabled = true)
    suspend fun openRequest(requestId: String): OpenedRequest = assistant.openRequest(requestId)

    /**
     * Denies one waiting request.
     *
     * The phone must be unlocked.
     *
     * @param requestId The id from listWaitingRequests.
     * @param note A short note for the agent, at most 500 characters.
     */
    @AppFunctionDeclaration(isDescribedByKDoc = true, isEnabled = false)
    @AppFunctionAccessLevel(level = AppFunctionMetadata.ACCESS_LEVEL_ANDROID_TRUSTED, isCompatEnforcementEnabled = true)
    suspend fun denyRequest(requestId: String, note: String? = null): DeniedRequest = assistant.denyRequest(requestId, note)

    /**
     * Denies every request that is waiting.
     *
     * Leaves pairing requests and other phones waiting. The phone must be unlocked.
     */
    @AppFunctionDeclaration(isDescribedByKDoc = true, isEnabled = false)
    @AppFunctionAccessLevel(level = AppFunctionMetadata.ACCESS_LEVEL_ANDROID_TRUSTED, isCompatEnforcementEnabled = true)
    suspend fun denyAllWaiting(): DeniedCount = assistant.denyAllWaiting()

    /**
     * Turns Lockdown on. Every request is denied until the user ends Lockdown in Reins.
     *
     * Works while the phone is locked. There is no function to turn Lockdown off.
     */
    @AppFunctionDeclaration(isDescribedByKDoc = true, isEnabled = false)
    @AppFunctionAccessLevel(level = AppFunctionMetadata.ACCESS_LEVEL_ANDROID_TRUSTED, isCompatEnforcementEnabled = true)
    suspend fun startLockdown(): ReinsStatus = assistant.startLockdown()

    /**
     * Stops Bypass, Overnight and Auto, and returns Reins to Manual so requests wait for the user.
     *
     * Agents set to Auto individually are set to Manual too. A connection in Lockdown is left as it is.
     * Refuses while Lockdown is on for everyone. Works while the phone is locked.
     */
    @AppFunctionDeclaration(isDescribedByKDoc = true, isEnabled = false)
    @AppFunctionAccessLevel(level = AppFunctionMetadata.ACCESS_LEVEL_ANDROID_TRUSTED, isCompatEnforcementEnabled = true)
    suspend fun switchToManual(): ReinsStatus = assistant.switchToManual()

    /**
     * Lists what connected AI agents did, newest first.
     *
     * Summaries are one line and name the operation only. Password vault and payment entries name the operation only.
     * The phone must be unlocked. This does not search message text.
     *
     * @param ai Only agents whose name contains this, ignoring case.
     * @param integration Only integrations whose name contains this, ignoring case.
     * @param since Include entries at or after this time.
     * @param limit How many to return. 15 when omitted or not positive. Never more than 50.
     */
    @AppFunctionDeclaration(isDescribedByKDoc = true, isEnabled = false)
    @AppFunctionAccessLevel(level = AppFunctionMetadata.ACCESS_LEVEL_ANDROID_TRUSTED, isCompatEnforcementEnabled = true)
    suspend fun searchActivity(ai: String? = null, integration: String? = null, since: Instant? = null, limit: Int? = null): List<ActivityItem> =
        assistant.searchActivity(ai, integration, since, limit)

    /**
     * Lists standing permissions that are still open.
     *
     * The phone must be unlocked.
     */
    @AppFunctionDeclaration(isDescribedByKDoc = true, isEnabled = false)
    @AppFunctionAccessLevel(level = AppFunctionMetadata.ACCESS_LEVEL_ANDROID_TRUSTED, isCompatEnforcementEnabled = true)
    suspend fun listStandingPermissions(): List<StandingPermission> = assistant.listStandingPermissions()

    /**
     * Revokes one standing permission so the agent has to ask again.
     *
     * The phone must be unlocked.
     *
     * @param permissionId The id from listStandingPermissions.
     */
    @AppFunctionDeclaration(isDescribedByKDoc = true, isEnabled = false)
    @AppFunctionAccessLevel(level = AppFunctionMetadata.ACCESS_LEVEL_ANDROID_TRUSTED, isCompatEnforcementEnabled = true)
    suspend fun revokeStandingPermission(permissionId: String): RevokedPermission = assistant.revokeStandingPermission(permissionId)

    /**
     * Lists AI agents connected to Reins.
     *
     * The phone must be unlocked. This does not disconnect an agent.
     */
    @AppFunctionDeclaration(isDescribedByKDoc = true, isEnabled = false)
    @AppFunctionAccessLevel(level = AppFunctionMetadata.ACCESS_LEVEL_ANDROID_TRUSTED, isCompatEnforcementEnabled = true)
    suspend fun listConnectedAis(): List<ConnectedAi> = assistant.listConnectedAis()
}

/** Reads and denies need the phone unlocked. The emergency actions do not call this. */
private fun deviceLocked(context: Context): Boolean =
    context.getSystemService(KeyguardManager::class.java)?.isDeviceLocked == true
