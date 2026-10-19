package dev.reins.android.ui.common

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.LiveRegionMode
import androidx.compose.ui.semantics.liveRegion
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.dp
import dev.reins.android.design.ButtonStyle
import dev.reins.android.design.CapsuleButton
import dev.reins.android.design.Glyph
import dev.reins.android.design.GlyphIcon
import dev.reins.android.design.LocalColors
import dev.reins.android.design.RText
import dev.reins.android.design.RType

/** A finishing surface, retained until acknowledged. Permission choices will use this same surface. */
@Composable
fun IntegrationCompletion(title: String, identity: String, onDone: () -> Unit, onAnother: (() -> Unit)? = null) {
    val c = LocalColors.current
    Column(
        Modifier.fillMaxWidth().padding(20.dp).testTag("integrationConnected").semantics { liveRegion = LiveRegionMode.Polite },
        verticalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        GlyphIcon(Glyph.Check, c.success, size = 30.dp)
        RText(title, RType.serif(27f), c.text)
        RText(untrusted(identity), RType.sans(17f, lineHeight = 24f), c.text, Modifier.testTag("connectedIdentity"), ltr = true)
        RText("The connection is saved. You can manage it here at any time.", RType.sans(14f, lineHeight = 20f), c.secondary)
        CapsuleButton("Done", Modifier.fillMaxWidth().testTag("connectionDone"), onClick = onDone)
        onAnother?.let {
            CapsuleButton("Add another account", Modifier.fillMaxWidth().testTag("connectionAddAnother"), style = ButtonStyle.Secondary, onClick = it)
        }
    }
}
