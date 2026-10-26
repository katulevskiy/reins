package dev.reins.android.design

import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.font.createFontFamilyResolver
import androidx.compose.ui.text.TextMeasurer
import androidx.compose.ui.unit.Density
import androidx.compose.ui.unit.LayoutDirection
import androidx.compose.ui.unit.sp
import androidx.test.core.app.ApplicationProvider
import androidx.test.ext.junit.runners.AndroidJUnit4
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode

/** Sora's weights resolve through the Cyrillic fallback chain: each weight sets the same words wider than the last. */
@RunWith(AndroidJUnit4::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [35])
class FontsTest {
    private fun width(weight: FontWeight, text: String = "Push 8 commits to octo/app"): Float {
        val context = ApplicationProvider.getApplicationContext<android.content.Context>()
        Fonts.init(context)
        val measurer = TextMeasurer(createFontFamilyResolver(context), Density(3f), LayoutDirection.Ltr)
        return measurer.measure(text, TextStyle(fontFamily = Fonts.sans, fontWeight = weight, fontSize = 20.sp)).size.width.toFloat()
    }

    @Test
    fun soraWeightsAreDistinct() {
        val regular = width(FontWeight.Normal)
        val medium = width(FontWeight.Medium)
        val semi = width(FontWeight.SemiBold)
        println("widths $regular $medium $semi")
        assertTrue("500 wider than 400 ($regular, $medium)", medium > regular)
        assertTrue("600 wider than 500 ($medium, $semi)", semi > medium)
    }

    @Test
    fun cyrillicIsDrawn() {
        // Without the fallback, Cyrillic letters would be tofu or the system face; either way it lays out.
        assertTrue(width(FontWeight.Medium, "Ваш ИИ спрашивает") > 0f)
    }
}
