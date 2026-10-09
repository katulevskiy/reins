package dev.reins.android.ui.vault

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateMapOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import dev.reins.android.design.Banner
import dev.reins.android.design.BannerKind
import dev.reins.android.design.ButtonStyle
import dev.reins.android.design.CapsuleButton
import dev.reins.android.design.Glyph
import dev.reins.android.design.LocalColors
import dev.reins.android.design.RText
import dev.reins.android.design.RTextField
import dev.reins.android.design.RType
import dev.reins.android.design.Screen
import dev.reins.android.ui.common.untrusted
import dev.reins.core.VaultFieldInput
import dev.reins.core.VaultItemDetail
import dev.reins.core.VaultItemInput
import dev.reins.core.VaultItemKind

/** One field of the form: its key in the core, its label, and how it is typed. */
data class FormField(
    val key: String,
    val label: String,
    val secret: Boolean = false,
    val multiline: Boolean = false,
    val keyboard: KeyboardType = KeyboardType.Text,
)

/** The fields the form offers for a kind; an API key is a login with only its key (in the password) and a website. */
fun formFields(kind: VaultItemKind, apiKey: Boolean): List<FormField> = when (kind) {
    VaultItemKind.LOGIN -> if (apiKey) {
        listOf(FormField("password", "API key", secret = true), FormField("uris", "Website (optional)", keyboard = KeyboardType.Uri))
    } else {
        listOf(
            FormField("username", "Username", keyboard = KeyboardType.Email),
            FormField("password", "Password", secret = true),
            FormField("uris", "Website", keyboard = KeyboardType.Uri),
            FormField("totp", "One-time code key (optional)", secret = true),
        )
    }
    VaultItemKind.NOTE -> listOf(FormField("notes", "Note", secret = true, multiline = true))
    VaultItemKind.CARD -> listOf(
        FormField("holder", "Name on the card"),
        FormField("number", "Number", secret = true, keyboard = KeyboardType.Number),
        FormField("exp_month", "Expiry month (1-12)", keyboard = KeyboardType.Number),
        FormField("exp_year", "Expiry year (2030)", keyboard = KeyboardType.Number),
        FormField("code", "Security code", secret = true, keyboard = KeyboardType.Number),
        FormField("brand", "Brand (optional)"),
    )
    VaultItemKind.IDENTITY -> listOf(
        FormField("first_name", "First name"),
        FormField("last_name", "Last name"),
        FormField("email", "Email", keyboard = KeyboardType.Email),
        FormField("phone", "Phone", keyboard = KeyboardType.Phone),
        FormField("address1", "Address"),
        FormField("city", "City"),
        FormField("state", "State or region"),
        FormField("postal_code", "Postal code"),
        FormField("country", "Country"),
        FormField("company", "Company (optional)"),
    )
    VaultItemKind.SSH_KEY -> emptyList()
}

/** What `vault:NAME/...` reference a new item of this kind gets, for the hint under its name. */
fun referenceHint(kind: VaultItemKind, name: String): String? {
    val shown = name.trim().ifEmpty { "NAME" }
    return when (kind) {
        VaultItemKind.LOGIN -> "Use it as vault:$shown/password"
        VaultItemKind.NOTE -> "Use it as vault:$shown/notes"
        VaultItemKind.SSH_KEY -> "The desktop app's SSH agent offers it"
        else -> null
    }
}

/**
 * The fields to send: everything filled in for a new item; for an edit, the plain fields that changed and the secret
 * fields typed again (an empty secret field means "unchanged").
 */
fun changedFields(fields: List<FormField>, values: Map<String, String>, original: Map<String, String>?): List<VaultFieldInput> =
    fields.mapNotNull { f ->
        val value = values[f.key].orEmpty()
        when {
            original == null -> value.takeIf { it.isNotBlank() }
            f.secret -> value.takeIf { it.isNotEmpty() }
            value != original[f.key].orEmpty() -> value
            else -> null
        }?.let { VaultFieldInput(f.key, it) }
    }

/** Adding an item ([newItem]) or changing one ([existing]). An SSH key is made on the phone, or pasted. */
@Composable
fun VaultEditScreen(
    viewModel: VaultViewModel,
    newItem: NewItem?,
    existing: VaultItemDetail?,
    authenticator: dev.reins.android.platform.Authenticator,
    onBack: () -> Unit,
    onSaved: (String) -> Unit,
) {
    val c = LocalColors.current
    val ui by viewModel.ui.collectAsStateWithLifecycle()
    val kind = existing?.kind ?: newItem?.kind ?: VaultItemKind.LOGIN
    val apiKey = newItem == NewItem.ApiKey
    val fields = remember(kind, apiKey) { formFields(kind, apiKey) }
    val original = remember(existing) { existing?.fields?.mapNotNull { f -> f.value?.let { f.key to it } }?.toMap() }
    var name by rememberSaveable { mutableStateOf(existing?.name.orEmpty()) }
    val values = remember(existing) { mutableStateMapOf<String, String>().apply { original?.let { putAll(it) } } }
    var pasteKey by rememberSaveable { mutableStateOf(false) }
    LaunchedEffect(Unit) { viewModel.clearError() }
    dev.reins.android.ui.common.SecureWindow()

    val title = when {
        existing != null -> "Edit"
        newItem == NewItem.ApiKey || newItem == NewItem.SshKey -> "New ${newItem.title}"
        newItem != null -> "New ${newItem.title.lowercase()}"
        else -> "New item"
    }
    Screen(title = title, subtitle = existing?.name?.let(::untrusted), onBack = onBack) {
        Column(Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(10.dp)) {
            Label("Name")
            RTextField(
                name, { name = it }, if (apiKey) "OpenAI" else "Name", tag = "vaultName", enabled = !ui.busy,
                keyboardOptions = KeyboardOptions(autoCorrectEnabled = false),
            )
            referenceHint(kind, name)?.let {
                RText(it, RType.mono(13f), c.secondary, Modifier.padding(horizontal = 4.dp).testTag("vaultHint"))
            }
            val sshNew = kind == VaultItemKind.SSH_KEY && existing == null
            fields.forEach { f -> FieldInput(f, values[f.key].orEmpty(), editing = existing != null, enabled = !ui.busy) { values[f.key] = it } }
            if (sshNew && pasteKey) {
                FieldInput(
                    FormField("private_key", "Private key (OpenSSH, without a passphrase)", secret = true, multiline = true),
                    values["private_key"].orEmpty(),
                    editing = false,
                    enabled = !ui.busy,
                ) { values["private_key"] = it }
            }
            ui.error?.let { Banner(untrusted(it), Modifier.padding(top = 4.dp), BannerKind.Error, tag = "vaultFormError") }
            when {
                sshNew && !pasteKey -> {
                    CapsuleButton(
                        "Make a new key on this phone",
                        Modifier.fillMaxWidth().padding(top = 6.dp).testTag("vaultGenerate"),
                        enabled = name.isNotBlank(),
                        busy = ui.busy,
                        glyph = Glyph.Key,
                    ) { viewModel.generateSshKey(authenticator, name.trim(), onSaved) }
                    RText(
                        "An Ed25519 key. You get its public half to put on servers; the private half never leaves your vault.",
                        RType.sans(13f, lineHeight = 18f),
                        c.secondary,
                        Modifier.padding(horizontal = 4.dp),
                    )
                    CapsuleButton("I have a key: paste it", Modifier.fillMaxWidth().testTag("vaultPasteKey"), style = ButtonStyle.Ghost) {
                        pasteKey = true
                    }
                }
                else -> {
                    val send = if (sshNew) {
                        listOfNotNull(values["private_key"]?.takeIf { it.isNotBlank() }?.let { VaultFieldInput("private_key", it) })
                    } else {
                        changedFields(fields, values, original)
                    }
                    val ready = name.isNotBlank() && (existing != null || send.isNotEmpty() || kind == VaultItemKind.IDENTITY)
                    CapsuleButton("Save", Modifier.fillMaxWidth().padding(top = 6.dp).testTag("vaultSave"), enabled = ready, busy = ui.busy) {
                        viewModel.save(authenticator, existing?.id, VaultItemInput(kind, name.trim(), send), onSaved)
                    }
                }
            }
        }
        RText(
            "Saved encrypted with your vault's key, on this phone, before it reaches the server.",
            RType.sans(13.5f, lineHeight = 19f),
            c.tertiary,
            Modifier.padding(start = 32.dp, end = 32.dp, top = 8.dp, bottom = 32.dp),
        )
    }
}

@Composable
private fun Label(text: String) {
    RText(text, RType.sans(13f, FontWeight.Medium), LocalColors.current.secondary, Modifier.padding(start = 4.dp, top = 6.dp))
}

@Composable
private fun FieldInput(field: FormField, value: String, editing: Boolean, enabled: Boolean, onChange: (String) -> Unit) {
    Label(field.label)
    RTextField(
        value,
        onChange,
        if (field.secret && editing) "Unchanged" else "",
        tag = "vaultInput:${field.key}",
        enabled = enabled,
        mono = field.secret || field.multiline,
        singleLine = !field.multiline,
        password = field.secret && !field.multiline,
        keyboardOptions = KeyboardOptions(
            // A password keyboard learns nothing, whether the secret is one line or a pasted key.
            keyboardType = if (field.secret) KeyboardType.Password else field.keyboard,
            autoCorrectEnabled = false,
        ),
    )
}
