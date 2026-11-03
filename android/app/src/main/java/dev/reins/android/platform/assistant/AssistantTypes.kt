package dev.reins.android.platform.assistant

import android.app.PendingIntent
import androidx.appfunctions.AppFunctionSerializable
import java.time.Instant

/** The mode Reins is in, and how much is waiting. */
@AppFunctionSerializable(isDescribedByKDoc = true)
data class ReinsStatus(
    /** manual, auto, bypass, overnight or lockdown. */
    val mode: String,
    /** When a timed Bypass or Overnight ends. Absent when the mode has no end. */
    val until: Instant?,
    /** How many requests are waiting for the user. */
    val waiting: Int,
    /** How many AI agents are connected. */
    val connectedAis: Int,
)

/** One request waiting for the user. */
@AppFunctionSerializable(isDescribedByKDoc = true)
data class WaitingRequest(
    /** The id to pass to openRequest or denyRequest. */
    val id: String,
    /** The connected AI agent that asked. */
    val ai: String,
    /** The integration the request is about, such as Gmail or GitHub. */
    val integration: String,
    /** What the request is, in one line. */
    val summary: String,
    /** When the request started waiting. */
    val waitingSince: Instant,
    /** When the agent stops waiting. Absent when it will wait. */
    val aiWaitsUntil: Instant?,
    /** Whether the user could approve it from the notification. Informational: this does not approve it. */
    val canApproveQuickly: Boolean,
)

/** A screen that opens one request in Reins. Approving happens there. */
@AppFunctionSerializable(isDescribedByKDoc = true)
data class OpenedRequest(
    /** Opens that request. The user approves or denies it in Reins. */
    val pendingIntent: PendingIntent,
)

/** A request that was denied. */
@AppFunctionSerializable(isDescribedByKDoc = true)
data class DeniedRequest(
    /** The request that was denied. */
    val requestId: String,
)

/** How many waiting requests were denied. */
@AppFunctionSerializable(isDescribedByKDoc = true)
data class DeniedCount(
    /** Waiting requests denied. Pairings and other phones are left waiting. */
    val count: Int,
)

/** One thing a connected AI agent did. */
@AppFunctionSerializable(isDescribedByKDoc = true)
data class ActivityItem(
    /** When it was decided. */
    val at: Instant,
    /** The connected AI agent. */
    val ai: String,
    /** The integration it concerned. */
    val integration: String,
    /** What it was, in one line. */
    val summary: String,
    /** approved, denied, error, revoked, held, undone, or the core's own word. */
    val outcome: String,
    /** you, autopilot, bypass, lockdown or standing permission. */
    val decidedBy: String,
)

/** A standing permission that is still open. */
@AppFunctionSerializable(isDescribedByKDoc = true)
data class StandingPermission(
    /** The id to pass to revokeStandingPermission. */
    val id: String,
    /** The connected AI agent it was given to. */
    val ai: String,
    /** The integration it covers. */
    val integration: String,
    /** What it allows, in one line. */
    val allows: String,
    /** When it ends. Absent when it has no end. */
    val expiresAt: Instant?,
)

/** A standing permission that was revoked. */
@AppFunctionSerializable(isDescribedByKDoc = true)
data class RevokedPermission(
    /** The permission that was revoked. */
    val permissionId: String,
)

/** An AI agent connected to Reins. */
@AppFunctionSerializable(isDescribedByKDoc = true)
data class ConnectedAi(
    /** The connection id. */
    val id: String,
    /** The name the user gave it. */
    val name: String,
    /** When it was last used. Absent when it has not been used. */
    val lastSeen: Instant?,
)
