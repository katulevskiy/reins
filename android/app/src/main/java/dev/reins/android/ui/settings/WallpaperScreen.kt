package dev.reins.android.ui.settings

import android.graphics.Bitmap
import android.graphics.BitmapFactory
import android.net.Uri
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.PickVisualMediaRequest
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.aspectRatio
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.offset
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.Slider
import androidx.compose.material3.SliderDefaults
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableFloatStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.draw.drawBehind
import androidx.compose.ui.geometry.CornerRadius
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.PathEffect
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.role
import androidx.compose.ui.semantics.selected
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import dev.reins.android.design.Appearance
import dev.reins.android.design.ButtonStyle
import dev.reins.android.design.Choice
import dev.reins.android.design.Chooser
import dev.reins.android.design.Glyph
import dev.reins.android.design.GlyphIcon
import dev.reins.android.design.LocalColors
import dev.reins.android.design.RText
import dev.reins.android.design.RType
import dev.reins.android.design.SignalButton
import dev.reins.android.design.Wallpaper
import dev.reins.android.design.Wallpapers
import dev.reins.android.design.pressable
import dev.reins.android.feedback.Event
import dev.reins.android.feedback.LocalFeedback
import dev.reins.android.feedback.play
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

/** A wallpaper as a small picture, cropped from its top: Activity's art at a glance. */
@Composable
fun WallpaperThumb(w: Wallpaper, dark: Boolean, modifier: Modifier = Modifier, radius: Dp = 16.dp) {
    val c = LocalColors.current
    val context = LocalContext.current
    val version = Wallpapers.photoVersion
    val image = remember(w, dark, version) { w.bitmap(context, dark)?.let { Bitmap.createScaledBitmap(it, 156, 200, true).asImageBitmap() } }
    Box(modifier.clip(RoundedCornerShape(radius)).background(c.paperSunken)) {
        if (image != null) Image(image, null, Modifier.fillMaxSize(), contentScale = ContentScale.Crop, alignment = Alignment.TopCenter)
    }
}

/**
 * Settings › Wallpaper (Next, Settings 04b and 04c): one for dark and one for light, chosen from the shipped art or the
 * owner's own photo. A photo gets an effect (as it is, dithered, scanlines, halftone or colour) and a fade into the
 * page, baked into the picture kept on this phone.
 */
@Composable
fun WallpaperScreen(onBack: () -> Unit) {
    val c = LocalColors.current
    val context = LocalContext.current
    val feedback = LocalFeedback.current
    val choice by Appearance.state(context)
    var forDark by remember { mutableStateOf(c.dark) }
    var picked by remember { mutableStateOf<Bitmap?>(null) }
    val scope = rememberCoroutineScope()
    val pick = rememberLauncherForActivityResult(ActivityResultContracts.PickVisualMedia()) { uri: Uri? ->
        if (uri != null) {
            scope.launch {
                picked = withContext(Dispatchers.IO) {
                    runCatching { context.contentResolver.openInputStream(uri)?.use { BitmapFactory.decodeStream(it) } }.getOrNull()
                }?.let { PhotoEffects.fit(it) }
            }
        }
    }
    val photo = picked
    if (photo != null) {
        YourPhoto(photo, forDark, onBack = { picked = null }) {
            Appearance.set(context, if (forDark) choice.copy(wallpaperDark = Wallpaper.Photo) else choice.copy(wallpaperLight = Wallpaper.Photo))
            picked = null
        }
        return
    }
    SettingsPage("Wallpaper", onBack = onBack) {
        Chooser(
            listOf(Choice(true, "Dark", "wallpaperFor:dark"), Choice(false, "Light", "wallpaperFor:light")),
            selected = forDark,
            onSelect = { forDark = it },
            modifier = Modifier.padding(horizontal = 20.dp).padding(top = 6.dp),
        )
        val current = choice.wallpaper(forDark)
        val tiles: List<Wallpaper?> = Wallpaper.shipped + listOf<Wallpaper?>(null)
        Column(Modifier.padding(horizontal = 16.dp).padding(top = 14.dp), verticalArrangement = Arrangement.spacedBy(12.dp)) {
            tiles.chunked(3).forEach { row ->
                Row(horizontalArrangement = Arrangement.spacedBy(10.dp)) {
                    row.forEach { w ->
                        Column(Modifier.weight(1f), horizontalAlignment = Alignment.CenterHorizontally) {
                            if (w == null) {
                                // The owner's own photo: a dashed tile that opens the picker.
                                Box(
                                    Modifier
                                        .fillMaxWidth()
                                        .aspectRatio(0.77f)
                                        .drawBehind {
                                            drawRoundRect(
                                                c.rule2,
                                                cornerRadius = CornerRadius(16.dp.toPx()),
                                                style = Stroke(1.5.dp.toPx(), pathEffect = PathEffect.dashPathEffect(floatArrayOf(6.dp.toPx(), 5.dp.toPx()))),
                                            )
                                        }
                                        .clip(RoundedCornerShape(16.dp))
                                        .pressable(shape = RoundedCornerShape(16.dp), label = "Your photo") {
                                            feedback.play(Event.Open)
                                            pick.launch(PickVisualMediaRequest(ActivityResultContracts.PickVisualMedia.ImageOnly))
                                        }
                                        .testTag("wallpaper:own"),
                                    contentAlignment = Alignment.Center,
                                ) {
                                    Column(horizontalAlignment = Alignment.CenterHorizontally) {
                                        GlyphIcon(Glyph.Plus, c.ink2, size = 22.dp)
                                        RText("Your photo", RType.sans(13f, FontWeight.SemiBold), c.ink2, Modifier.padding(top = 6.dp), maxLines = 1)
                                    }
                                }
                            } else {
                                val on = w == current
                                Box(
                                    Modifier
                                        .fillMaxWidth()
                                        .aspectRatio(0.77f)
                                        .then(if (on) Modifier.border(2.dp, c.violet, RoundedCornerShape(16.dp)).padding(3.dp) else Modifier.border(1.dp, c.rule, RoundedCornerShape(16.dp)))
                                        .clip(RoundedCornerShape(if (on) 13.dp else 16.dp))
                                        .pressable(shape = RoundedCornerShape(16.dp), label = w.label) {
                                            feedback.play(Event.Selection)
                                            Appearance.set(context, if (forDark) choice.copy(wallpaperDark = w) else choice.copy(wallpaperLight = w))
                                        }
                                        .semantics {
                                            role = Role.RadioButton
                                            selected = on
                                        }
                                        .testTag("wallpaper:${w.name}"),
                                ) {
                                    WallpaperThumb(w, forDark, Modifier.fillMaxSize(), radius = if (on) 13.dp else 16.dp)
                                    if (on) {
                                        Box(
                                            Modifier.align(Alignment.BottomEnd).padding(8.dp).size(22.dp).clip(CircleShape).background(c.violetFill),
                                            contentAlignment = Alignment.Center,
                                        ) { GlyphIcon(Glyph.Check, Color.White, size = 15.dp) }
                                    }
                                }
                                RText(w.label, RType.sans(12.5f, if (on) FontWeight.SemiBold else FontWeight.Normal), if (on) c.ink else c.ink2, Modifier.padding(top = 6.dp), maxLines = 1, align = TextAlign.Center)
                            }
                        }
                    }
                    repeat(3 - row.size) { Spacer(Modifier.weight(1f)) }
                }
            }
        }
    }
}

/**
 * Your photo (Next, Settings 04c): the photo as Activity would show it, five effects to pick from and how far it fades
 * into the page; "Use for dark" (or light) bakes it and keeps it on this phone.
 */
@Composable
private fun YourPhoto(photo: Bitmap, dark: Boolean, onBack: () -> Unit, onUse: () -> Unit) {
    val c = LocalColors.current
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    var effect by remember { mutableStateOf(PhotoEffect.Dither) }
    var fade by remember { mutableFloatStateOf(0.6f) }
    var preview by remember { mutableStateOf<Bitmap?>(null) }
    val paper = c.paper
    val ink = if (dark) Color(0xFFE6E8FF) else Color(0xFF2A2932)
    LaunchedEffect(photo, effect, fade, dark) {
        preview = withContext(Dispatchers.Default) { PhotoEffects.apply(PhotoEffects.small(photo), effect, fade, paper, ink) }
    }
    val previews = remember(photo, dark) {
        PhotoEffect.entries.associateWith { PhotoEffects.apply(PhotoEffects.small(photo, 120), it, 0f, paper, ink).asImageBitmap() }
    }
    androidx.activity.compose.BackHandler(onBack = onBack)
    SettingsPage("Your photo", onBack = onBack) {
        Box(Modifier.fillMaxWidth().padding(top = 8.dp), contentAlignment = Alignment.Center) {
            // The page as it would look: the art at the top, the title over it, panes under it.
            Box(Modifier.width(220.dp).height(282.dp).clip(RoundedCornerShape(26.dp)).background(paper).testTag("photoPreview")) {
                preview?.let { Image(it.asImageBitmap(), null, Modifier.fillMaxSize(), contentScale = ContentScale.Crop, alignment = Alignment.TopCenter) }
                RText("Activity", RType.display(26f), if (dark) Color.White else c.ink, Modifier.padding(start = 14.dp, top = 14.dp), maxLines = 1)
                Column(Modifier.align(Alignment.BottomCenter).padding(10.dp).offset(y = 0.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                    repeat(4) { Box(Modifier.fillMaxWidth().height(28.dp).clip(RoundedCornerShape(10.dp)).background(c.paperRaised)) }
                }
            }
        }
        Row(Modifier.fillMaxWidth().padding(horizontal = 16.dp).padding(top = 16.dp), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            PhotoEffect.entries.forEach { e ->
                val on = e == effect
                Column(Modifier.weight(1f), horizontalAlignment = Alignment.CenterHorizontally) {
                    Box(
                        Modifier
                            .fillMaxWidth()
                            .aspectRatio(1.05f)
                            .then(if (on) Modifier.border(2.dp, c.violet, RoundedCornerShape(14.dp)).padding(3.dp) else Modifier)
                            .clip(RoundedCornerShape(if (on) 11.dp else 14.dp))
                            .pressable(shape = RoundedCornerShape(14.dp), label = e.label) { effect = e }
                            .semantics {
                                role = Role.RadioButton
                                selected = on
                            }
                            .testTag("effect:${e.name}"),
                    ) { previews[e]?.let { Image(it, null, Modifier.fillMaxSize(), contentScale = ContentScale.Crop) } }
                    RText(e.label, RType.sans(12f, if (on) FontWeight.SemiBold else FontWeight.Normal), if (on) c.ink else c.ink2, Modifier.padding(top = 4.dp), maxLines = 1)
                }
            }
        }
        Row(Modifier.fillMaxWidth().padding(horizontal = 20.dp).padding(top = 8.dp), verticalAlignment = Alignment.CenterVertically) {
            GlyphIcon(Glyph.Appearance, c.inkMeta, size = 16.dp)
            Slider(
                fade,
                onValueChange = { fade = it },
                modifier = Modifier.weight(1f).padding(horizontal = 10.dp).semantics { contentDescription = "Fade" }.testTag("photoFade"),
                colors = SliderDefaults.colors(thumbColor = Color.White, activeTrackColor = c.violetFill, inactiveTrackColor = c.paperSunken),
            )
            RText("Fade", RType.sans(13f), c.inkMeta, maxLines = 1)
        }
        SignalButton(
            if (dark) "Use for dark" else "Use for light",
            Modifier.fillMaxWidth().padding(horizontal = 20.dp).padding(top = 24.dp).testTag("usePhoto"),
            style = ButtonStyle.Inverse,
            glyph = Glyph.Check,
        ) {
            scope.launch {
                withContext(Dispatchers.IO) {
                    val baked = PhotoEffects.apply(photo, effect, fade, paper, ink)
                    Wallpapers.photoFile(context, dark).outputStream().use { baked.compress(Bitmap.CompressFormat.JPEG, 88, it) }
                }
                Wallpapers.photoChanged()
                onUse()
            }
        }
    }
}
