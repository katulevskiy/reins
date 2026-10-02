package dev.rewarden.android.design.blobatar

import java.text.Normalizer
import java.util.Locale
import kotlin.math.max
import kotlin.math.min

/**
 * Blobatar 2 (gen2), https://github.com/Alain00/blobatar, MIT licence, (c) 2026 Alain.
 *
 * This is a line-for-line port of the library's static renderer (`hash`, `traits`, `color`,
 * `shape`, `styles/shapes`, `styles/compose`, `render`) because the library is TypeScript and
 * nothing on Android runs it. It adds no geometry and changes no constant; `BlobatarGoldenTest`
 * renders the library's own 1000-seed golden fixture and requires byte-identical SVG markup.
 * Trig and pow go through [StrictMath] (fdlibm), the same algorithms V8 uses, for the same reason.
 *
 * Expressions, motion and trait overrides of the library are not ported: the app draws the resting figure.
 */
object Blobatar {
    /** One drawn primitive; the fill rides on every mark. */
    sealed interface Mark {
        val fill: String

        data class Path(val d: String, override val fill: String) : Mark

        data class Circle(val cx: Double, val cy: Double, val r: Double, override val fill: String) : Mark
    }

    /** The figure: the backdrop plate (if any) followed by the marks, in draw order, in a 100 × 100 box. */
    data class Figure(val backdrop: Mark.Path?, val marks: List<Mark>)

    enum class Backdrop { NONE, SQUARE, CIRCLE, SQUIRCLE }

    fun figure(name: String, backdrop: Backdrop = Backdrop.NONE): Figure {
        val t = Traits(name)
        val palette = palette(t.num("hue", 0.0, 360.0), true, t.unit("tone"))
        val layout = layout(t)
        val plate = when (backdrop) {
            Backdrop.NONE -> null
            Backdrop.SQUARE -> Mark.Path("M0 0H100V100H0Z", palette.bg)
            Backdrop.CIRCLE -> Mark.Path(superellipse(Ellipse(50.0, 50.0, 50.0, 50.0), 2.0, 0.0), palette.bg)
            Backdrop.SQUIRCLE -> Mark.Path(superellipse(Ellipse(50.0, 50.0, 50.0, 50.0), 6.0, 0.0), palette.bg)
        }
        return Figure(plate, marks(layout, palette))
    }

    /** The library's `blobatar(name, { background })` markup, used to check this port against the golden fixture. */
    fun svg(name: String, backdrop: Backdrop = Backdrop.NONE): String {
        val figure = figure(name, backdrop)
        val head = figure.marks.first { it is Mark.Path || it is Mark.Circle }.fill
        val eye = figure.marks.last().fill
        val body = StringBuilder()
        figure.backdrop?.let { body.append("<path d=\"${it.d}\" fill=\"${it.fill}\"/>") }
        body.append("<g fill=\"$head\">")
        val eyes = 2
        val bodyMarks = figure.marks.dropLast(eyes)
        for (m in bodyMarks) {
            when (m) {
                is Mark.Circle -> body.append("<circle cx=\"${num(m.cx)}\" cy=\"${num(m.cy)}\" r=\"${num(m.r)}\"/>")
                is Mark.Path -> body.append("<path d=\"${m.d}\"/>")
            }
        }
        body.append("</g><g fill=\"$eye\">")
        for (m in figure.marks.takeLast(eyes)) body.append("<path d=\"${(m as Mark.Path).d}\"/>")
        body.append("</g>")
        return "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 100 100\">$body</svg>"
    }

    // ---- hash ---------------------------------------------------------------------------------------------

    private const val SEP = 0xff

    private fun feed(state: Int, bytes: ByteArray): Int {
        var h = state
        for (b in bytes) {
            h = (h xor (b.toInt() and 0xff)) * 3432918353L.toInt()
            h = (h shl 13) or (h ushr 19)
        }
        return h
    }

    private fun finalize(state: Int): Long {
        var h = state
        h = (h xor (h ushr 16)) * 2246822507L.toInt()
        h = (h xor (h ushr 13)) * 3266489909L.toInt()
        return (h xor (h ushr 16)).toLong() and 0xffffffffL
    }

    /** JavaScript's `String.prototype.trim` whitespace set (white space and line terminators, incl. U+FEFF). */
    private fun isJsSpace(c: Char): Boolean =
        c == '﻿' || (c != '​' && (Character.isSpaceChar(c) || c in '\t'..'\r'))

    fun normalizeSeed(seed: String): String =
        Normalizer.normalize(seed, Normalizer.Form.NFC).trim(::isJsSpace).lowercase(Locale.ROOT)

    private class Traits(seed: String) {
        private val state: Int

        init {
            val s = normalizeSeed(seed)
            state = feed(1779033703 xor s.length, s.toByteArray(Charsets.UTF_8))
        }

        /** Uniform float in [0, 1). */
        fun unit(key: String): Double =
            finalize(feed(feed(state, byteArrayOf(SEP.toByte())), key.toByteArray(Charsets.UTF_8))) / 4294967296.0

        fun num(key: String, min: Double, max: Double) = min + unit(key) * (max - min)

        fun int(key: String, min: Int, max: Int) = min + Math.floor(unit(key) * (max - min + 1)).toInt()

        fun jitter(key: String, amount: Double) = (unit(key) * 2 - 1) * amount
    }

    // ---- colour -------------------------------------------------------------------------------------------

    private data class Oklch(val l: Double, val c: Double, val h: Double)

    private class Palette(val bg: String, val head: String, val eye: String)

    private fun toLinear(color: Oklch): DoubleArray {
        val r = color.h * Math.PI / 180
        val a = color.c * StrictMath.cos(r)
        val b = color.c * StrictMath.sin(r)
        val l1 = color.l + 0.3963377774 * a + 0.2158037573 * b
        val m1 = color.l - 0.1055613458 * a - 0.0638541728 * b
        val s1 = color.l - 0.0894841775 * a - 1.291485548 * b
        val l = l1 * l1 * l1
        val m = m1 * m1 * m1
        val s = s1 * s1 * s1
        return doubleArrayOf(
            4.0767416621 * l - 3.3077115913 * m + 0.2309699292 * s,
            -1.2684380046 * l + 2.6097574011 * m - 0.3413193965 * s,
            -0.0041960863 * l - 0.7034186147 * m + 1.707614701 * s,
        )
    }

    private fun inGamut(rgb: DoubleArray) = rgb.all { it >= -1e-4 && it <= 1 + 1e-4 }

    private fun resolve(color: Oklch): DoubleArray {
        var rgb = toLinear(color)
        if (!inGamut(rgb)) {
            var lo = 0.0
            var hi = color.c
            repeat(12) {
                val mid = (lo + hi) / 2
                if (inGamut(toLinear(color.copy(c = mid)))) lo = mid else hi = mid
            }
            rgb = toLinear(color.copy(c = lo))
        }
        return DoubleArray(3) { min(1.0, max(0.0, rgb[it])) }
    }

    private fun luminance(color: Oklch): Double {
        val (r, g, b) = resolve(color).toList()
        return 0.2126 * r + 0.7152 * g + 0.0722 * b
    }

    private fun contrast(a: Oklch, b: Oklch): Double {
        val x = luminance(a)
        val y = luminance(b)
        return (max(x, y) + 0.05) / (min(x, y) + 0.05)
    }

    private fun ensureContrast(fg: Oklch, bg: Oklch, min: Double): Oklch {
        if (contrast(fg, bg) >= min) return fg
        val lean = if (fg.l >= bg.l) 1 else -1
        for (dir in intArrayOf(lean, -lean)) {
            var probe = fg
            for (i in 0 until 60) {
                probe = probe.copy(l = min(1.0, max(0.0, probe.l + dir * 0.02)))
                if (contrast(probe, bg) >= min) return probe
                if (probe.l == 0.0 || probe.l == 1.0) break
            }
        }
        val black = fg.copy(l = 0.0, c = 0.0)
        val white = fg.copy(l = 1.0, c = 0.0)
        return if (contrast(black, bg) >= contrast(white, bg)) black else white
    }

    private fun toHex(color: Oklch): String {
        val sb = StringBuilder("#")
        for (v in resolve(color)) {
            val s = if (v <= 0.0031308) 12.92 * v else 1.055 * StrictMath.pow(v, 1 / 2.4) - 0.055
            sb.append(Math.round(s * 255).toString(16).padStart(2, '0'))
        }
        return sb.toString()
    }

    private val TONES = listOf(
        0.2 to Oklch(0.86, 0.085, 0.0),
        0.36 to Oklch(0.9, 0.028, 0.0),
        0.62 to Oklch(0.73, 0.135, 0.0),
        0.8 to Oklch(0.62, 0.165, 0.0),
        0.93 to Oklch(0.87, 0.16, 0.0),
        1.0 to Oklch(0.34, 0.035, 0.0),
    )

    private val DARK_SURFACE = Oklch(0.145, 0.0, 0.0)
    private const val SURFACE_FLOOR = 1.5

    private fun palette(hue: Double, enforce: Boolean, tone: Double): Palette {
        val t = (TONES.firstOrNull { tone < it.first } ?: TONES.first()).second
        val head0 = ensureContrast(Oklch(t.l, t.c, hue), DARK_SURFACE, SURFACE_FLOOR)
        val bg = Oklch(0.965, 0.01, hue)
        var head = head0
        var eye = if (head0.l >= 0.5) Oklch(0.17, 0.02, hue) else Oklch(0.97, 0.012, hue)
        if (enforce) {
            head = ensureContrast(head, bg, 1.25)
            eye = ensureContrast(eye, head, 4.5)
        }
        return Palette(toHex(bg), toHex(head), toHex(eye))
    }

    // ---- shape primitives ---------------------------------------------------------------------------------

    /** JavaScript's `String(Math.round(v * 100) / 100)` (with -0 as "0"). */
    private fun num(v: Double): String {
        val scaled = Math.round(v * 100)
        if (scaled == 0L) return "0"
        val abs = Math.abs(scaled)
        val whole = abs / 100
        val frac = (abs % 100).toInt()
        val sign = if (scaled < 0) "-" else ""
        return when {
            frac == 0 -> "$sign$whole"
            frac % 10 == 0 -> "$sign$whole.${frac / 10}"
            else -> "$sign$whole.${frac.toString().padStart(2, '0')}"
        }
    }

    private fun r2(v: Double) = Math.round(v * 100) / 100.0

    private data class Ellipse(val cx: Double, val cy: Double, val rx: Double, val ry: Double)

    private fun superellipse(e: Ellipse, n: Double = 4.0, rot: Double = 0.0): String {
        val k = min(1.0, (8 * StrictMath.pow(2.0, -1 / n) - 4) / 3)
        val a = e.rx
        val b = e.ry
        val ak = a * k
        val bk = b * k
        val pts = arrayOf(
            a to 0.0, a to bk, ak to b, 0.0 to b, -ak to b, -a to bk, -a to 0.0,
            -a to -bk, -ak to -b, 0.0 to -b, ak to -b, a to -bk, a to 0.0,
        )
        val t = rot * Math.PI / 180
        val cos = StrictMath.cos(t)
        val sin = StrictMath.sin(t)
        fun at(i: Int): String {
            val (x, y) = pts[i]
            return "${num(e.cx + x * cos - y * sin)} ${num(e.cy + x * sin + y * cos)}"
        }
        val d = StringBuilder("M${at(0)}")
        var i = 1
        while (i < 13) {
            d.append("C${at(i)} ${at(i + 1)} ${at(i + 2)}")
            i += 3
        }
        return d.append("Z").toString()
    }

    private fun blobPath(cx: Double, cy: Double, rx: Double, ry: Double, radii: List<Double>, rot: Double): String {
        val n = radii.size
        val t0 = rot * Math.PI / 180
        val p = radii.mapIndexed { i, m ->
            val a = t0 + 2 * Math.PI * i / n
            (cx + rx * m * StrictMath.cos(a)) to (cy + ry * m * StrictMath.sin(a))
        }
        fun at(i: Int) = p[((i % n) + n) % n]
        val d = StringBuilder("M${num(at(0).first)} ${num(at(0).second)}")
        for (i in 0 until n) {
            val (x0, y0) = at(i - 1)
            val (x1, y1) = at(i)
            val (x2, y2) = at(i + 1)
            val (x3, y3) = at(i + 2)
            d.append("C${num(x1 + (x2 - x0) / 6)} ${num(y1 + (y2 - y0) / 6)}")
            d.append(" ${num(x2 - (x3 - x1) / 6)} ${num(y2 - (y3 - y1) / 6)}")
            d.append(" ${num(x2)} ${num(y2)}")
        }
        return d.append("Z").toString()
    }

    private fun polygon(b: Body): String {
        val sides = b.sides
        val round = b.round
        val k = if (round > 0) (if (round < 1) round / 2 else 0.5) else 0.0
        val t0 = b.rot * Math.PI / 180 - Math.PI / 2
        val v = List(sides) { i ->
            val a = t0 + 2 * Math.PI * i / sides
            (b.cx + b.rx * StrictMath.cos(a)) to (b.cy + b.ry * StrictMath.sin(a))
        }
        fun at(i: Int) = v[((i % sides) + sides) % sides]
        fun cut(i: Int, j: Int): String {
            val (x0, y0) = at(i)
            val (x1, y1) = at(j)
            return "${num(x0 + (x1 - x0) * k)} ${num(y0 + (y1 - y0) * k)}"
        }
        val d = StringBuilder("M${cut(0, -1)}")
        for (i in 0 until sides) {
            val (x, y) = at(i)
            d.append("Q${num(x)} ${num(y)} ${cut(i, i + 1)}")
            if (k < 0.5) d.append("L${cut(i + 1, i)}")
        }
        return d.append("Z").toString()
    }

    private fun box(cx: Double, cy: Double, rx: Double, ry: Double): String {
        val l = num(cx - rx)
        val r = num(cx + rx)
        return "M$l ${num(cy - ry)}H${r}V${num(cy + ry)}H${l}Z"
    }

    private fun taper(cx: Double, cy: Double, rx: Double, ry: Double, tip: Double): String {
        val t = max(1.05, tip)
        val tx = rx * StrictMath.sqrt(1 - 1 / (t * t))
        val ty = cy - ry / t
        val apex = cy - t * ry
        val px = tx * 0.14
        val py = ty + 0.86 * (apex - ty)
        return "M${num(cx - tx)} ${num(ty)}" +
            "L${num(cx - px)} ${num(py)}" +
            "Q${num(cx)} ${num(apex)} ${num(cx + px)} ${num(py)}" +
            "L${num(cx + tx)} ${num(ty)}Z"
    }

    // ---- silhouettes and layout ---------------------------------------------------------------------------

    private class Body(
        var cx: Double, var cy: Double, var rx: Double, var ry: Double,
        var n: Double, var rot: Double, val radii: List<Double>,
    ) {
        var sides = 0
        var round = 0.0

        fun ellipse() = Ellipse(cx, cy, rx, ry)
    }

    private class Petal(val cx: Double, val cy: Double, val r: Double)

    private class Deco {
        val petals = ArrayList<Petal>()
        val extra = ArrayList<String>()
    }

    private class Shape(
        val name: String,
        val core: Double,
        val body: ((Traits, Body) -> Unit)? = null,
        val face: ((Body) -> Ellipse)? = null,
        val decorate: ((Traits, Body, Deco) -> Unit)? = null,
        val path: ((Body) -> String)? = null,
    )

    private val spline: (Body) -> String = { blobPath(it.cx, it.cy, it.rx, it.ry, it.radii, it.rot) }

    private fun shrunk(k: Double): (Body) -> Ellipse = { Ellipse(it.cx, it.cy, it.rx * k, it.ry * k) }

    private val splineFace: (Body) -> Ellipse = { shrunk(it.radii.min() * 0.95)(it) }
    private val polyFace: (Body) -> Ellipse = shrunk(0.84)

    private val round = Shape("round", 1.0)
    private val organic = Shape("organic", 0.98, path = spline, face = splineFace)
    private val boxy = Shape(
        "boxy", 0.86,
        body = { t, b ->
            b.n = t.num("body.n", 3.4, 6.0)
            b.rot = t.num("body.rot", -20.0, 20.0)
        },
    )
    private val capsule = Shape(
        "capsule", 1.02,
        body = { t, b -> b.ry *= t.num("capsule.squat", 0.55, 0.68) },
        face = shrunk(0.94),
        decorate = { _, b, out ->
            for (s in intArrayOf(-1, 1)) out.petals.add(Petal(b.cx + s * (b.rx - b.ry), b.cy, b.ry))
        },
        path = { box(it.cx, it.cy, it.rx - it.ry, it.ry) },
    )
    private val nub = Shape(
        "nub", 0.88,
        decorate = { t, b, out ->
            val count = t.int("nub.n", 1, 2)
            for (i in 0 until count) {
                val a = t.num("nub.a$i", 0.0, 2 * Math.PI)
                out.petals.add(
                    Petal(
                        b.cx + StrictMath.cos(a) * b.rx * 0.88,
                        b.cy + StrictMath.sin(a) * b.rx * 0.88,
                        b.rx * t.num("nub.r$i", 0.24, 0.4),
                    ),
                )
            }
        },
    )
    private val cloud = Shape(
        "cloud", 0.78, face = splineFace, path = spline,
        decorate = { t, b, out ->
            val count = t.int("cloud.n", 4, 6)
            for (i in 0 until count) {
                val a = Math.PI + (Math.PI * (i + 0.5)) / count
                out.petals.add(
                    Petal(
                        b.cx + StrictMath.cos(a) * b.rx * 0.8,
                        b.cy + StrictMath.sin(a) * b.rx * 0.5,
                        b.rx * t.num("cloud.r$i", 0.44, 0.62),
                    ),
                )
            }
        },
    )
    private val droplet = Shape(
        "droplet", 0.78,
        body = { _, b ->
            b.cy += 0.22 * b.ry
            b.n = 2.0
        },
        face = { Ellipse(it.cx, it.cy + it.ry * 0.05, it.rx * 0.88, it.ry * 0.88) },
        decorate = { t, b, out -> out.extra.add(taper(b.cx, b.cy, b.rx, b.ry, t.num("droplet.tip", 1.4, 1.65))) },
    )
    private val hexagon = Shape(
        "hexagon", 1.05, path = ::polygon, face = polyFace,
        body = { t, b ->
            b.sides = 6
            b.rot = t.num("body.rot", -12.0, 12.0)
            b.round = t.num("poly.round", 0.24, 0.5)
        },
    )
    private val sun = Shape(
        "sun", 0.7,
        decorate = { t, b, out ->
            val count = t.int("sun.n", 6, 9)
            val dist = b.rx * t.num("sun.dist", 1.0, 1.08)
            val pr = b.rx * t.num("sun.r", 0.2, 0.26)
            val off = t.num("sun.rot", 0.0, 2 * Math.PI)
            for (i in 0 until count) {
                val a = off + (2 * Math.PI * i) / count
                out.petals.add(Petal(b.cx + StrictMath.cos(a) * dist, b.cy + StrictMath.sin(a) * dist, pr))
            }
        },
    )
    private val triangle = Shape(
        "triangle", 1.15, path = ::polygon,
        body = { t, b ->
            b.sides = 3
            b.rot = t.num("body.rot", -5.0, 5.0)
            b.round = t.num("poly.round", 0.24, 0.5)
        },
        face = { Ellipse(it.cx, it.cy + it.ry * 0.1, it.rx * 0.54, it.ry * 0.36) },
    )

    /** The gen2 band table: `[shape, upper edge of its band in [0, 1)]`. */
    private val BANDS = listOf(
        round to 0.22, organic to 0.48, boxy to 0.6, capsule to 0.7, nub to 0.79,
        cloud to 0.86, droplet to 0.915, hexagon to 0.95, sun to 0.98, triangle to 1.0,
    )

    private class Eye(val cx: Double, val cy: Double, val rx: Double, val ry: Double, val n: Double, val rot: Double)

    private class Layout(
        val shape: Shape,
        val body: Body,
        val petals: List<Petal>,
        val extra: List<String>,
        val eyes: List<Eye>,
    )

    private fun faceFit(t: Traits, b: Body, face: Ellipse): List<Eye> {
        val rx = b.rx
        val er0 = t.num("eye.rx", 0.075, 0.105) * rx
        val ratio = t.num("eye.ratio", 1.9, 3.2)
        val scale = t.num("eye.scale", 0.78, 1.24)
        val stretch = t.num("eye.stretch", 0.85, 1.18)
        val clearance = t.num("eye.gap", 0.1, 0.24) * rx
        val wide = er0 * max(1.0, scale)
        val tall = er0 * ratio * max(1.0, scale * stretch)
        val gap0 = wide + rx * 0.03 + clearance

        val gx = t.jitter("gaze.x", 0.09) * face.rx
        val gy = t.num("gaze.y", -0.2, 0.08) * face.ry
        val dy = t.jitter("eye.dy", 0.04) * face.ry
        val reach = StrictMath.hypot(wide, tall)
        val need = StrictMath.hypot(
            (Math.abs(gx) + gap0 + reach) / face.rx,
            (Math.abs(gy) + Math.abs(dy) + reach) / face.ry,
        )
        val fit = if (need > 0.9) 0.9 / need else 1.0

        val er = er0 * fit
        val eyeRy = er * ratio
        val gap = gap0 * fit
        val room = max(0.0, min(1.0, clearance / tall))
        val bound = min(12.0, (StrictMath.asin(room) * 180) / Math.PI)
        val lean = t.num("eye.lean", -1.0, 1.0) * bound
        val lean2 = max(-12.0, min(12.0, lean + t.jitter("eye.lean2", 3.5)))

        val cx = face.cx + gx * fit
        val cy = face.cy + gy * fit
        return listOf(
            Eye(cx - gap, cy, er, eyeRy, t.num("eye.n", 3.5, 6.0), lean),
            Eye(cx + gap, cy + dy * fit, er * scale, eyeRy * scale * stretch, t.num("eye.n", 3.5, 6.0), lean2),
        )
    }

    private fun layout(t: Traits): Layout {
        val v = t.unit("shape")
        val shape = (BANDS.firstOrNull { v < it.second } ?: BANDS.last()).first
        val r = t.num("body.r", 31.0, 38.0) * shape.core
        val body = Body(
            cx = 50 + t.jitter("body.x", 1.5),
            cy = 50 + t.jitter("body.y", 1.5),
            rx = r,
            ry = r * t.num("body.ratio", 0.92, 1.08),
            n = t.num("body.n", 1.9, 2.5),
            rot = 0.0,
            radii = List(t.int("body.pts", 6, 8)) { i -> 1 + t.jitter("body.r$i", 0.16) },
        )
        shape.body?.invoke(t, body)
        val face = shape.face?.invoke(body) ?: body.ellipse()
        val deco = Deco()
        shape.decorate?.invoke(t, body, deco)
        return Layout(shape, body, deco.petals, deco.extra, faceFit(t, body, face))
    }

    private fun marks(l: Layout, p: Palette): List<Mark> = buildList {
        for (d in l.petals) add(Mark.Circle(r2(d.cx), r2(d.cy), r2(d.r), p.head))
        for (d in l.extra) add(Mark.Path(d, p.head))
        add(Mark.Path(l.shape.path?.invoke(l.body) ?: superellipse(l.body.ellipse(), l.body.n, l.body.rot), p.head))
        for (e in l.eyes) add(Mark.Path(superellipse(Ellipse(e.cx, e.cy, e.rx, e.ry), e.n, e.rot), p.eye))
    }
}
