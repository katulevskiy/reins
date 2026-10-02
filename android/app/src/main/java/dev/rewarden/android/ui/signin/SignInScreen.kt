package dev.rewarden.android.ui.signin

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.KeyboardCapitalization
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import dev.rewarden.android.BuildConfig
import dev.rewarden.android.design.Banner
import dev.rewarden.android.design.BannerKind
import dev.rewarden.android.design.ButtonStyle
import dev.rewarden.android.design.CapsuleButton
import dev.rewarden.android.design.Glyph
import dev.rewarden.android.design.GlyphIcon
import dev.rewarden.android.design.LocalColors
import dev.rewarden.android.design.RText
import dev.rewarden.android.design.RTextField
import dev.rewarden.android.design.RType
import dev.rewarden.android.design.Screen

@Composable
fun SignInScreen(viewModel: SignInViewModel) {
    val ui by viewModel.ui.collectAsStateWithLifecycle()
    val c = LocalColors.current
    var server by remember { mutableStateOf(BuildConfig.DEFAULT_SERVER) }
    var email by remember { mutableStateOf("") }
    var password by remember { mutableStateOf("") }
    var totp by remember { mutableStateOf("") }

    Screen(title = null) {
        Column(Modifier.padding(horizontal = 24.dp), verticalArrangement = Arrangement.spacedBy(12.dp)) {
            Spacer(Modifier.height(56.dp))
            Box(
                Modifier.size(56.dp).background(c.accentSoft, RoundedCornerShape(18.dp)),
                contentAlignment = Alignment.Center,
            ) { GlyphIcon(Glyph.ShieldCheck, c.accent, size = 30.dp) }
            Spacer(Modifier.height(6.dp))
            RText("Rewarden", RType.sans(34f, FontWeight.SemiBold), c.text)
            RText(
                "Sign in with your Rewarden account. This phone approves what your AI assistants ask for.",
                RType.sans(15.5f, lineHeight = 22f),
                c.secondary,
            )
            Spacer(Modifier.height(10.dp))
            RTextField(
                server, { server = it }, "Server", tag = "server", enabled = !ui.busy,
                keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Uri, autoCorrectEnabled = false),
            )
            RTextField(
                email, { email = it }, "Email", tag = "email", enabled = !ui.busy,
                keyboardOptions = KeyboardOptions(
                    keyboardType = KeyboardType.Email,
                    autoCorrectEnabled = false,
                    capitalization = KeyboardCapitalization.None,
                ),
            )
            RTextField(
                password, { password = it }, "Master password", tag = "password", enabled = !ui.busy, password = true,
                keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Password, autoCorrectEnabled = false),
            )
            if (ui.needsTotp) {
                RTextField(
                    totp, { totp = it }, "Two-factor code", tag = "totp", enabled = !ui.busy, mono = true,
                    keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Number, autoCorrectEnabled = false),
                )
            }
            ui.error?.let { Banner(it, kind = BannerKind.Error) }
            Spacer(Modifier.height(4.dp))
            CapsuleButton(
                "Sign in",
                Modifier.fillMaxWidth().testTag("signIn"),
                style = ButtonStyle.Primary,
                enabled = server.length > "https://".length && email.isNotBlank() && password.isNotEmpty(),
                busy = ui.busy,
            ) { viewModel.signIn(server, email, password, totp) }
            Spacer(Modifier.height(24.dp))
        }
    }
}
