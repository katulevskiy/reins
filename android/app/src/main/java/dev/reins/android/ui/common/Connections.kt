package dev.reins.android.ui.common

import androidx.compose.runtime.Composable
import androidx.compose.runtime.compositionLocalOf
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import dev.reins.android.design.ConnectionAvatar
import dev.reins.core.ConnectionView

/** The AI connections, so any screen can show a connection's chosen icon. */
val LocalConnections = compositionLocalOf<List<ConnectionView>> { emptyList() }

/**
 * The connection an entry belongs to: by id, or, for entries logged before ids were recorded (an empty id), by the
 * label it had then, when exactly one connection carries it.
 */
fun findConnection(connections: List<ConnectionView>, connectionId: String, label: String): ConnectionView? =
    connections.firstOrNull { it.id == connectionId }
        ?: if (connectionId.isEmpty()) connections.singleOrNull { it.label == label } else null

/** A connection's icon by id: the one it shows everywhere, or, for one that is gone, the icon its [label] suggests. */
@Composable
fun ConnectionIcon(connectionId: String, label: String, modifier: Modifier = Modifier, size: Dp = 40.dp) {
    val connection = findConnection(LocalConnections.current, connectionId, label)
    if (connection != null) {
        ConnectionAvatar(connection, modifier, size)
    } else {
        ConnectionAvatar(untrusted(label), null, modifier, size)
    }
}
