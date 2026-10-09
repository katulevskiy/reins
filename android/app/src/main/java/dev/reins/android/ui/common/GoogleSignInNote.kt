package dev.reins.android.ui.common

import androidx.compose.foundation.layout.padding
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.unit.dp
import dev.reins.android.design.LocalColors
import dev.reins.android.design.RText
import dev.reins.android.design.RType

/**
 * Whether Google has verified Reins's use of Gmail, Calendar and Contacts. Until it has, Google's consent screen warns
 * that the app is not verified; set this once the verification is through and the note below goes away.
 */
const val GOOGLE_APP_VERIFIED = false

/** What adding a Google account looks like, said before the button so Google's screens surprise no one. */
@Composable
fun GoogleSignInNote(modifier: Modifier = Modifier) {
    val text = if (GOOGLE_APP_VERIFIED) {
        "Google asks which account, then what Reins may do with it."
    } else {
        "Google asks which account, then what Reins may do with it. While Google reviews Reins, it first warns " +
            "\"Google hasn't verified this app\": tap Advanced, then Go to Reins."
    }
    RText(text, RType.sans(13.5f, lineHeight = 19f), LocalColors.current.secondary, modifier.padding(horizontal = 4.dp).testTag("googleSignInNote"))
}
