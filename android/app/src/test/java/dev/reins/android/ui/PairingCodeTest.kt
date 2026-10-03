package dev.reins.android.ui

import dev.reins.android.ui.pairing.PairingCode
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

class PairingCodeTest {
    @Test
    fun `a bare code is normalised whatever its case, spaces and dashes`() {
        assertEquals("BCDF-GHJK", PairingCode.parse("BCDF-GHJK"))
        assertEquals("BCDF-GHJK", PairingCode.parse("bcdfghjk"))
        assertEquals("BCDF-GHJK", PairingCode.parse("  bcdf ghjk "))
        assertEquals("BCDF-GHJK", PairingCode.parse("B-C-D-F-G-H-J-K"))
        assertEquals("ZXWV-TSRQ", PairingCode.normalize("zxwv\ttsrq"))
    }

    @Test
    fun `only the eight letters of the alphabet make a code`() {
        assertNull(PairingCode.parse("BCDF-GHJ"))
        assertNull(PairingCode.parse("BCDF-GHJKL"))
        // Vowels, Y and digits are not in the alphabet.
        assertNull(PairingCode.parse("ABCD-FGHJ"))
        assertNull(PairingCode.parse("BCDF-GHJY"))
        assertNull(PairingCode.parse("BCDF-GH1K"))
        assertNull(PairingCode.parse("BCDF_GHJK"))
        assertNull(PairingCode.parse(""))
        assertNull(PairingCode.parse("   "))
        assertNull(PairingCode.parse(null))
        // Look-alike letters from other scripts are refused, not folded.
        assertNull(PairingCode.parse("BCDF-GHJК"))
        assertNull(PairingCode.parse("bcdf-ghjſ"))
    }

    @Test
    fun `the official link and a self-hosted server's own link carry the code`() {
        assertEquals("BCDF-GHJK", PairingCode.parse("https://app.reins2fa.com/pair?code=BCDF-GHJK"))
        assertEquals("BCDF-GHJK", PairingCode.parse("https://app.reins2fa.com/pair/?code=bcdf-ghjk"))
        assertEquals("BCDF-GHJK", PairingCode.parse("https://reins.example.com/pair?code=BCDFGHJK"))
        assertEquals("BCDF-GHJK", PairingCode.parse("https://example.com/reins/pair?source=qr&code=BCDF-GHJK"))
        assertEquals("BCDF-GHJK", PairingCode.parse("http://192.168.1.20:8000/pair?code=BCDF%2DGHJK"))
        assertEquals("BCDF-GHJK", PairingCode.parse("HTTPS://APP.REINS2FA.COM/pair?code=BCDF-GHJK"))
    }

    @Test
    fun `the reins scheme carries the code`() {
        assertEquals("BCDF-GHJK", PairingCode.parse("reins://pair?code=BCDF-GHJK"))
        assertEquals("BCDF-GHJK", PairingCode.parse("reins://pair/?code=bcdf+ghjk"))
        assertNull(PairingCode.parse("reins://other?code=BCDF-GHJK"))
        assertNull(PairingCode.parse("reins://pair/more?code=BCDF-GHJK"))
        assertNull(PairingCode.parse("reins:pair?code=BCDF-GHJK"))
    }

    @Test
    fun `links that are not pairing links are refused`() {
        assertNull(PairingCode.parse("https://app.reins2fa.com/login?code=BCDF-GHJK"))
        assertNull(PairingCode.parse("https://app.reins2fa.com/pairing?code=BCDF-GHJK"))
        assertNull(PairingCode.parse("https://app.reins2fa.com/pair"))
        assertNull(PairingCode.parse("https://app.reins2fa.com/pair?other=BCDF-GHJK"))
        assertNull(PairingCode.parse("https://app.reins2fa.com/pair?code=ABCD-EFGH"))
        assertNull(PairingCode.parse("https:///pair?code=BCDF-GHJK"))
        assertNull(PairingCode.parse("ftp://app.reins2fa.com/pair?code=BCDF-GHJK"))
        assertNull(PairingCode.parse("javascript:alert(1)//pair?code=BCDF-GHJK"))
        assertNull(PairingCode.parse("intent://pair?code=BCDF-GHJK#Intent;scheme=reins;end"))
        assertNull(PairingCode.parse("https://app.reins2fa.com/pair?code=BCDF-GHJK and more"))
    }

    @Test
    fun `a link with two codes is not one the server made`() {
        assertNull(PairingCode.parse("https://app.reins2fa.com/pair?code=BCDF-GHJK&code=ZXWV-TSRQ"))
    }

    @Test
    fun `very long input is not looked at`() {
        assertNull(PairingCode.parse("https://app.reins2fa.com/pair?pad=" + "x".repeat(3000) + "&code=BCDF-GHJK"))
    }
}
