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
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.aspectRatio
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyListState
import androidx.compose.foundation.lazy.LazyRow
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.Slider
import androidx.compose.material3.SliderDefaults
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableFloatStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.produceState
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.draw.drawBehind
import androidx.compose.ui.geometry.CornerRadius
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.ImageBitmap
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
import dev.reins.android.design.Choice
import dev.reins.android.design.Chooser
import dev.reins.android.design.Glyph
import dev.reins.android.design.GlyphIcon
import dev.reins.android.design.LocalColors
import dev.reins.android.design.PhotoEffect
import dev.reins.android.design.PhotoEffects
import dev.reins.android.design.RColors
import dev.reins.android.design.RText
import dev.reins.android.design.RType
import dev.reins.android.design.Wallpaper
import dev.reins.android.design.WallpaperChoice
import dev.reins.android.design.Wallpapers
import dev.reins.android.design.pressable
import dev.reins.android.design.rememberWallpaper
import dev.reins.android.feedback.Event
import dev.reins.android.feedback.LocalFeedback
import dev.reins.android.feedback.play
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

/** A wallpaper as a small picture, cropped from its top: Activity's art at a glance, with its effect and fade. */
@Composable
fun WallpaperThumb(choice: WallpaperChoice, dark: Boolean, modifier: Modifier = Modifier, radius: Dp = 16.dp) {
    val c = LocalColors.current
    val art = rememberWallpaper(choice, dark)
    val image = remember(art) { art?.let { Bitmap.createScaledBitmap(it.bitmap, 156, 200, true).asImageBitmap() } }
    Box(modifier.clip(RoundedCornerShape(radius)).background(c.paperSunken)) {
        if (image != null) Image(image, null, Modifier.fillMaxSize(), contentScale = ContentScale.Crop, alignment = Alignment.TopCenter)
    }
}

/**
 * Settings › Wallpaper (Next, Settings 04b and 04c): one for dark and one for light. Three separate choices for each:
 * the image (the shipped art or the owner's own photo), its effect, and how far it fades into the page. They apply as
 * they are picked; picking an image gives it its own effect ([Wallpaper.defaultEffect]), which can then change. Every
 * image has the Effect row. The owner's photo is kept as picked, so its effect and fade can change any time.
 */
@Composable
fun WallpaperScreen(onBack: () -> Unit) {
    val c = LocalColors.current
    val context = LocalContext.current
    val feedback = LocalFeedback.current
    val choice by Appearance.state(context)
    var forDark by remember { mutableStateOf(c.dark) }
    val scope = rememberCoroutineScope()
    val version = Wallpapers.photoVersion
    val current = choice.wallpaper(forDark)
    val hasPhoto = remember(forDark, version) { Wallpapers.hasPhoto(context, forDark) }
    fun choose(w: WallpaperChoice) = Appearance.setWallpaper(context, forDark, w)
    val pick = rememberLauncherForActivityResult(ActivityResultContracts.PickVisualMedia()) { uri: Uri? ->
        if (uri != null) {
            val dark = forDark
            scope.launch {
                val saved = withContext(Dispatchers.IO) {
                    runCatching {
                        val src = context.contentResolver.openInputStream(uri)?.use { BitmapFactory.decodeStream(it) } ?: return@runCatching false
                        Wallpapers.savePhoto(context, dark, PhotoEffects.fit(src))
                        true
                    }.getOrDefault(false)
                }
                if (saved) Appearance.setWallpaper(context, dark, Appearance.state(context).value.wallpaper(dark).withNewPhoto())
            }
        }
    }
    val importPhoto = {
        feedback.play(Event.Open)
        pick.launch(PickVisualMediaRequest(ActivityResultContracts.PickVisualMedia.ImageOnly))
    }

    // The fade follows the finger in the preview and is kept when it lifts.
    var fade by remember(forDark, current.fadeStep) { mutableFloatStateOf(current.fade) }
    val shown = current.copy(fade = fade)

    SettingsPage("Wallpaper", onBack = onBack) {
        Chooser(
            listOf(Choice(true, "Dark", "wallpaperFor:dark"), Choice(false, "Light", "wallpaperFor:light")),
            selected = forDark,
            onSelect = { forDark = it },
            modifier = Modifier.padding(horizontal = 20.dp).padding(top = 6.dp),
        )
        Preview(shown, forDark, version)

        Label("Image")
        val images: List<Wallpaper> = Wallpaper.shipped + Wallpaper.Photo
        val listState = remember(forDark) { LazyListState(firstVisibleItemIndex = (images.indexOf(current.image) - 1).coerceAtLeast(0)) }
        LazyRow(
            state = listState,
            contentPadding = PaddingValues(horizontal = 16.dp),
            horizontalArrangement = Arrangement.spacedBy(10.dp),
        ) {
            items(images, key = { it.name }) { w ->
                val on = w == current.image
                Column(Modifier.width(84.dp), horizontalAlignment = Alignment.CenterHorizontally) {
                    if (w == Wallpaper.Photo && !hasPhoto) {
                        // No photo yet: a dashed tile that opens the picker.
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
                                .pressable(shape = RoundedCornerShape(16.dp), label = w.label, onClick = importPhoto)
                                .testTag("wallpaper:own"),
                            contentAlignment = Alignment.Center,
                        ) { GlyphIcon(Glyph.Plus, c.ink2, size = 22.dp) }
                    } else {
                        Box(Modifier.fillMaxWidth().aspectRatio(0.77f)) {
                            Picked(on, w.label, "wallpaper:${w.name}", radius = 16.dp, onClick = {
                                feedback.play(Event.Selection)
                                if (w != current.image) choose(current.withImage(w))
                            }) { r -> WallpaperThumb(WallpaperChoice.of(w), forDark, Modifier.fillMaxSize(), radius = r) }
                            if (w == Wallpaper.Photo) {
                                // The owner's photo: another one in its place.
                                Box(
                                    Modifier
                                        .align(Alignment.TopEnd)
                                        .padding(6.dp)
                                        .size(26.dp)
                                        .clip(CircleShape)
                                        .background(c.paperRaised)
                                        .pressable(shape = CircleShape, label = "Replace photo", onClick = importPhoto)
                                        .semantics { contentDescription = "Replace photo" }
                                        .testTag("wallpaper:own"),
                                    contentAlignment = Alignment.Center,
                                ) { GlyphIcon(Glyph.Pencil, c.ink2, size = 14.dp) }
                            }
                        }
                    }
                    RText(w.label, RType.sans(12.5f, if (on) FontWeight.SemiBold else FontWeight.Normal), if (on) c.ink else c.ink2, Modifier.padding(top = 6.dp), maxLines = 1, align = TextAlign.Center)
                }
            }
        }

        EffectRow(current, forDark, version) { e ->
            feedback.play(Event.Selection)
            choose(current.copy(effect = e))
        }

        Label("Fade")
        Slider(
            fade,
            onValueChange = { fade = it },
            onValueChangeFinished = { Appearance.setWallpaper(context, forDark, Appearance.state(context).value.wallpaper(forDark).copy(fade = fade)) },
            modifier = Modifier.fillMaxWidth().padding(horizontal = 20.dp).semantics { contentDescription = "Fade" }.testTag("wallpaperFade"),
            colors = SliderDefaults.colors(thumbColor = Color.White, activeTrackColor = c.violetFill, inactiveTrackColor = c.paperSunken),
        )
    }
}

/** The Effect row: the image with each effect, small; the chosen one ringed. */
@Composable
private fun EffectRow(current: WallpaperChoice, forDark: Boolean, version: Int, onPick: (PhotoEffect) -> Unit) {
    val c = LocalColors.current
    val context = LocalContext.current
    Label("Effect")
    val effects by produceState<Map<PhotoEffect, ImageBitmap>>(emptyMap(), current.image, forDark, version) {
        value = withContext(Dispatchers.Default) {
            // Each effect on the picture it is drawn on: a treated image's own effect is its shipped file.
            val smalls = HashMap<Boolean, Bitmap?>()
            PhotoEffect.entries.mapNotNull { e ->
                val choice = WallpaperChoice(current.image, e)
                val small = smalls.getOrPut(choice.fromShipped) { Wallpapers.base(context, choice, forDark)?.let { PhotoEffects.small(it, 120) } } ?: return@mapNotNull null
                e to (if (choice.applied == PhotoEffect.Original) small else PhotoEffects.apply(small, choice.applied, 0f, forDark, current.image.foot)).asImageBitmap()
            }.toMap()
        }
    }
    Row(Modifier.fillMaxWidth().padding(horizontal = 16.dp), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
        PhotoEffect.entries.forEach { e ->
            val on = e == current.effect
            Column(Modifier.weight(1f), horizontalAlignment = Alignment.CenterHorizontally) {
                Box(Modifier.fillMaxWidth().aspectRatio(0.77f)) {
                    Picked(on, e.label, "effect:${e.name}", radius = 14.dp, onClick = { onPick(e) }) { r ->
                        Box(Modifier.fillMaxSize().clip(RoundedCornerShape(r)).background(c.paperSunken)) {
                            effects[e]?.let { Image(it, null, Modifier.fillMaxSize(), contentScale = ContentScale.Crop, alignment = Alignment.TopCenter) }
                        }
                    }
                }
                RText(e.label, RType.sans(12f, if (on) FontWeight.SemiBold else FontWeight.Normal), if (on) c.ink else c.ink2, Modifier.padding(top = 4.dp), maxLines = 1)
            }
        }
    }
}

/** A quiet label over a row of choices. */
@Composable
private fun Label(text: String) {
    val c = LocalColors.current
    RText(text, RType.sans(12.5f, FontWeight.SemiBold, tracking = 0.015f), c.inkMeta, Modifier.padding(start = 20.dp, end = 20.dp, top = 22.dp, bottom = 8.dp), maxLines = 1)
}

/** A choice tile: a violet ring and a check when [on]; [content] is drawn with the corner it gets. */
@Composable
private fun Picked(on: Boolean, label: String, tag: String, radius: Dp, onClick: () -> Unit, content: @Composable (Dp) -> Unit) {
    val c = LocalColors.current
    val inner = if (on) radius - 3.dp else radius
    Box(
        Modifier
            .fillMaxSize()
            .then(if (on) Modifier.border(2.dp, c.violet, RoundedCornerShape(radius)).padding(3.dp) else Modifier.border(1.dp, c.rule, RoundedCornerShape(radius)))
            .clip(RoundedCornerShape(inner))
            .pressable(shape = RoundedCornerShape(radius), label = label, onClick = onClick)
            .semantics {
                role = Role.RadioButton
                selected = on
            }
            .testTag(tag),
    ) {
        content(inner)
        if (on) {
            Box(
                Modifier.align(Alignment.BottomEnd).padding(6.dp).size(20.dp).clip(CircleShape).background(c.violetFill),
                contentAlignment = Alignment.Center,
            ) { GlyphIcon(Glyph.Check, Color.White, size = 14.dp) }
        }
    }
}

/**
 * The page as it would look (Next, Settings 04c): the picture at the top under the title's scrim, panes under it,
 * in the chosen theme's paper. Made small and off the main thread, so the effect and the fade show as they change.
 */
@Composable
private fun Preview(choice: WallpaperChoice, dark: Boolean, version: Int) {
    val context = LocalContext.current
    val pc = if (dark) RColors.Dark else RColors.Light
    val base = remember(choice.image, choice.fromShipped, dark, version) { Wallpapers.base(context, choice, dark)?.let { PhotoEffects.small(it) } }
    var picture by remember { mutableStateOf<ImageBitmap?>(null) }
    LaunchedEffect(base, choice.effect, choice.fadeStep, dark) {
        picture = base?.let { b ->
            if (choice.plain) b.asImageBitmap() else withContext(Dispatchers.Default) { PhotoEffects.apply(b, choice.applied, choice.fadeStep / 100f, dark, choice.image.foot).asImageBitmap() }
        }
    }
    Box(Modifier.fillMaxWidth().padding(top = 14.dp), contentAlignment = Alignment.Center) {
        Box(Modifier.width(200.dp).height(256.dp).clip(RoundedCornerShape(24.dp)).background(pc.paper).testTag("wallpaperPreview")) {
            picture?.let { Image(it, null, Modifier.fillMaxSize(), contentScale = ContentScale.Crop, alignment = Alignment.TopCenter) }
            Box(
                Modifier
                    .fillMaxWidth()
                    .height(60.dp)
                    .background(Brush.verticalGradient(listOf(if (dark) Color.Black.copy(alpha = 0.55f) else pc.paper.copy(alpha = 0.75f), Color.Transparent))),
            )
            RText("Activity", RType.display(24f), if (dark) Color.White else pc.ink, Modifier.padding(start = 14.dp, top = 14.dp), maxLines = 1)
            Column(Modifier.align(Alignment.BottomCenter).padding(10.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                repeat(4) { Box(Modifier.fillMaxWidth().height(26.dp).clip(RoundedCornerShape(10.dp)).background(pc.paperRaised)) }
            }
        }
    }
}
