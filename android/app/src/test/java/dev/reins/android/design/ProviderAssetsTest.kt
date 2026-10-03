package dev.rewarden.android.design

import android.graphics.Color
import androidx.test.core.app.ApplicationProvider
import androidx.test.ext.junit.runners.AndroidJUnit4
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode

/** Every provider offered as an icon has its real logo bundled, and it really draws something. */
@RunWith(AndroidJUnit4::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [35])
class ProviderAssetsTest {
    private val context get() = ApplicationProvider.getApplicationContext<android.content.Context>()

    @Test
    fun `every provider has a logo that renders`() {
        for (provider in Providers.all) {
            val bitmap = SvgAssets.bitmap(context, provider.asset, 64)
            assertNotNull("no logo for ${provider.key}", bitmap)
            var painted = 0
            for (x in 0 until 64) for (y in 0 until 64) if (Color.alpha(bitmap!!.getPixel(x, y)) > 0) painted++
            assertTrue("${provider.key} drew nothing", painted > 64)
        }
    }

    @Test
    fun `the gmail mark renders too`() {
        assertNotNull(SvgAssets.bitmap(context, "services/gmail.svg", 48))
    }

    @Test
    fun `the new git hosts and mcp servers have marks that render`() {
        for (service in listOf("gitlab", "codeberg", "bitbucket", "mcp")) {
            val bitmap = SvgAssets.bitmap(context, "services/$service.svg", 64)
            assertNotNull("no mark for $service", bitmap)
            var painted = 0
            for (x in 0 until 64) for (y in 0 until 64) if (Color.alpha(bitmap!!.getPixel(x, y)) > 0) painted++
            assertTrue("$service drew nothing", painted > 64)
        }
    }

    @Test
    fun `names suggest their provider`() {
        assertEquals("gemini", Providers.infer("Gemini CLI")?.key)
        assertEquals("perplexity", Providers.infer("perplexity")?.key)
        assertEquals("mistral", Providers.infer("Le Chat")?.key)
        assertEquals("deepseek", Providers.infer("my deepseek")?.key)
        assertEquals("cursor", Providers.infer("Cursor IDE")?.key)
        assertEquals(Providers.all.size, Providers.all.map { it.key }.toSet().size)
    }
}
