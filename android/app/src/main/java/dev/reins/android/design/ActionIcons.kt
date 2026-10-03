package dev.reins.android.design

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.offset
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.rounded.Send
import androidx.compose.material.icons.automirrored.rounded.List
import androidx.compose.material.icons.rounded.Edit
import androidx.compose.material.icons.rounded.Key
import androidx.compose.material.icons.rounded.Link
import androidx.compose.material.icons.rounded.MarkEmailRead
import androidx.compose.material.icons.rounded.People
import androidx.compose.material.icons.rounded.PhoneAndroid
import androidx.compose.material.icons.rounded.Search
import androidx.compose.material.icons.rounded.UploadFile
import androidx.compose.material.icons.rounded.VerifiedUser
import androidx.compose.material3.Icon
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.alpha
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp

/** What an operation does. Each kind has its own icon and colour so lists read at a glance. */
enum class ActionKind(val key: String) {
    Search("search"),
    List("list"),
    Read("read"),
    Write("write"),
    Send("send"),
    Grant("grant"),
    Accounts("accounts"),
    Pair("pair"),
    /** Another phone of the account asks for its keys. */
    Join("join"),
    /** A file an AI uploaded through the server. */
    Upload("upload"),
    Other("request");

    val icon: ImageVector
        get() = when (this) {
            Search -> Icons.Rounded.Search
            List -> Icons.AutoMirrored.Rounded.List
            Write -> Icons.Rounded.Edit
            Read -> Icons.Rounded.MarkEmailRead
            Send -> Icons.AutoMirrored.Rounded.Send
            Grant -> Icons.Rounded.VerifiedUser
            Accounts -> Icons.Rounded.People
            Pair -> Icons.Rounded.Link
            Join -> Icons.Rounded.PhoneAndroid
            Upload -> Icons.Rounded.UploadFile
            Other -> Icons.Rounded.Key
        }

    fun tint(c: RColors): Color = when (this) {
        Search -> c.search
        List -> c.search
        Write -> c.send
        Read -> c.read
        Send -> c.send
        Grant -> c.grant
        Accounts -> c.accounts
        Pair -> c.pair
        Join -> c.pair
        Upload -> c.read
        Other -> c.secondary
    }

    companion object {
        fun of(action: String): ActionKind = entries.firstOrNull { it.key == action } ?: Other
    }
}

/**
 * The operation's icon on a tinted tile. An operation that covers several things (five emails, three recipients)
 * shows a stack of icons and how many.
 */
@Composable
fun ActionTile(action: ActionKind, count: Int, modifier: Modifier = Modifier, size: Dp = 44.dp, tint: Color? = null) {
    val c = LocalColors.current
    val color = tint ?: action.tint(c)
    val stacked = count > 1
    Box(modifier.size(size)) {
        Box(
            Modifier
                .size(size)
                .background(color.copy(alpha = 0.13f), RoundedCornerShape(size * 0.32f)),
            contentAlignment = Alignment.Center,
        ) {
            val icon = size * 0.52f
            if (stacked) {
                Icon(action.icon, null, Modifier.size(icon).offset(x = (-size * 0.13f), y = (-size * 0.13f)).alpha(0.28f), tint = color)
                Icon(action.icon, null, Modifier.size(icon).offset(x = (-size * 0.065f), y = (-size * 0.065f)).alpha(0.55f), tint = color)
            }
            Icon(action.icon, null, Modifier.size(icon).then(if (stacked) Modifier.offset(x = size * 0.03f, y = size * 0.03f) else Modifier), tint = color)
        }
        if (stacked) {
            Box(
                Modifier
                    .align(Alignment.BottomEnd)
                    .offset(x = size * 0.14f, y = size * 0.14f)
                    .background(color, CircleShape)
                    .padding(horizontal = size * 0.11f, vertical = size * 0.02f),
                contentAlignment = Alignment.Center,
            ) {
                RText(
                    if (count > 99) "99+" else count.toString(),
                    RType.sans((size.value * 0.27f).coerceAtLeast(10f), FontWeight.SemiBold),
                    Color.White,
                    maxLines = 1,
                )
            }
        }
    }
}

/** A small purple/blue number pill, used on nav items. */
@Composable
fun CountPill(count: Int, tint: Color, modifier: Modifier = Modifier) {
    if (count <= 0) return
    Box(
        modifier.background(tint, CircleShape).padding(horizontal = 6.dp, vertical = 1.dp),
        contentAlignment = Alignment.Center,
    ) {
        RText(if (count > 99) "99+" else count.toString(), RType.sans(11f, FontWeight.SemiBold), Color.White, maxLines = 1)
    }
}
