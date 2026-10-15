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

/**
 * What adding a Google account looks like, said before the button so Google's screens surprise no one: a short line
 * with the way through behind its (?).
 */
@Composable
fun GoogleSignInNote(modifier: Modifier = Modifier) {
    if (GOOGLE_APP_VERIFIED) return
    dev.reins.android.design.InlineHelp(
        "Google's warning",
        "If Google says it hasn't verified this app: Advanced, then Go to Reins.",
        modifier.padding(horizontal = 16.dp).testTag("googleSignInNote"),
        label = "Google may warn you first",
    )
}
