package io.hookecho.HookEcho

import org.json.JSONArray
import org.json.JSONObject
import java.io.File
import java.net.HttpURLConnection
import java.net.URL
import java.util.Locale

/**
 * The watched-points alert lookup, shared by the background service and the home-screen widget.
 *
 * Kotlin-only and stateless on purpose: `api.weather.gov/alerts/active?point=` answers "what is
 * warned here" with no geometry work, and the watch list is read from the same `settings.json` the
 * Rust app writes — a shared file rather than an IPC channel, since neither caller can assume the
 * app is running.
 */
object Nws {
    const val TIER_WATCH = 1
    const val TIER_WARNING = 2
    const val TIER_EMERGENCY = 3
    private const val USER_AGENT = "hookecho (github.com/d4vid87/hookecho)"

    /**
     * A watched place: [lat]/[lon] is where a notification flies the camera, [samples] is every
     * point asked about. A marker with a watch radius contributes its rim as well as its centre,
     * and a drawn zone contributes its vertices, so a warning that stops down the road is still
     * a warning here.
     */
    data class Watch(
        val name: String,
        val lat: Double,
        val lon: Double,
        val samples: List<DoubleArray>,
        val home: Boolean = false,
        val requestedSamples: Int = samples.size,
    )

    data class Alert(val id: String, val event: String, val headline: String, val tier: Int)
    data class AlertResponse(val alerts: List<Alert>, val error: String? = null) {
        val successful: Boolean get() = error == null
    }

    fun speechEnabled(filesDir: File): Boolean {
        val f = File(filesDir, "config/settings.json")
        val root = runCatching { JSONObject(f.readText()) }.getOrNull() ?: return true
        return root.optBoolean("speak_warnings", true) && !root.optBoolean("mute_alerts", false)
    }

    /**
     * Is the phone inside the user's quiet-hours window? Same `settings.json` fields, and the
     * same reading as the desktop: start == end is no window rather than all day, and a window
     * whose end is before its start wraps midnight.
     *
     * The escalated tier is not filtered here — callers let it through regardless, which is the
     * point of the tier.
     */
    fun inQuietHours(filesDir: File, hour: Int): Boolean {
        val f = File(filesDir, "config/settings.json")
        if (!f.exists()) return false
        val root = runCatching { JSONObject(f.readText()) }.getOrNull() ?: return false
        if (!root.optBoolean("quiet_hours", false)) return false
        val s = Math.floorMod(root.optInt("quiet_start_hour", 22), 24)
        val e = Math.floorMod(root.optInt("quiet_end_hour", 7), 24)
        val h = Math.floorMod(hour, 24)
        return when {
            s == e -> false
            s < e -> h in s until e
            else -> h >= s || h < e
        }
    }

    /** Rim points around a marker, so `?point=` sees a warning that only reaches the radius. */
    private const val RIM_POINTS = 6

    /** Vertices sampled per drawn zone, evenly spaced around its ring. */
    private const val ZONE_POINTS = 8

    /** Ceiling on `?point=` requests per pass; api.weather.gov is not ours to hammer. */
    private const val SAMPLE_CAP = 40

    /**
     * Saved markers and drawn zones from settings.json — the same file the Rust app reads and
     * writes (`markers[].name/lat/lon/alert_radius_mi`, `alert_polygons[].name/ring`).
     *
     * ponytail: sampled points, not geometry. `?point=` resolves zone-only alerts (geometry null)
     * server-side, which polygon math in Kotlin would have to re-implement; the ceiling is a small
     * warning threading between two samples. Real polygon intersection is the upgrade path.
     */
    fun watched(filesDir: File): List<Watch> {
        val f = File(filesDir, "config/settings.json")
        if (!f.exists()) return emptyList()
        val root = JSONObject(f.readText())
        val out = ArrayList<Watch>()

        val markers = root.optJSONArray("markers")
        for (i in 0 until (markers?.length() ?: 0)) {
            val m = markers?.optJSONObject(i) ?: continue
            val lat = m.optDouble("lat")
            val lon = m.optDouble("lon")
            if (!lat.isFinite() || !lon.isFinite()) continue
            val radiusMi = m.optDouble("alert_radius_mi", 0.0).let { if (it.isFinite()) it else 0.0 }
            val home = m.optBoolean("home", false)
            val samples = ArrayList<DoubleArray>()
            samples.add(doubleArrayOf(lat, lon))
            if (home && radiusMi != 30.0) {
                for (k in 0 until RIM_POINTS) samples.add(offset(lat, lon, 30.0, k * 360.0 / RIM_POINTS))
            }
            if (radiusMi > 0.0) {
                for (k in 0 until RIM_POINTS) samples.add(offset(lat, lon, radiusMi, k * 360.0 / RIM_POINTS))
            }
            out.add(Watch(m.optString("name", "Saved location"), lat, lon, samples, home))
        }

        val zones = root.optJSONArray("alert_polygons")
        for (i in 0 until (zones?.length() ?: 0)) {
            val z = zones?.optJSONObject(i) ?: continue
            val ring = z.optJSONArray("ring") ?: continue
            val pts = (0 until ring.length()).mapNotNull { k ->
                val p = ring.optJSONArray(k) ?: return@mapNotNull null
                val lon = p.optDouble(0)
                val lat = p.optDouble(1)
                if (lat.isFinite() && lon.isFinite()) doubleArrayOf(lat, lon) else null
            }
            if (pts.isEmpty()) continue
            val lat = pts.sumOf { it[0] } / pts.size
            val lon = pts.sumOf { it[1] } / pts.size
            val step = maxOf(1, pts.size / ZONE_POINTS)
            val samples = ArrayList<DoubleArray>()
            samples.add(doubleArrayOf(lat, lon))
            for (k in pts.indices step step) samples.add(pts[k])
            out.add(Watch(z.optString("name", "Watch zone"), lat, lon, samples))
        }

        // Cover every centre before spending requests on rims; retain unchecked places so the
        // result can disclose incomplete coverage instead of silently calling it all quiet.
        val ordered = out.sortedByDescending { it.home }
        val counts = IntArray(ordered.size)
        var budget = SAMPLE_CAP
        for (sample in 0 until (ordered.maxOfOrNull { it.samples.size } ?: 0)) {
            for ((index, watch) in ordered.withIndex()) {
                if (budget > 0 && sample < watch.samples.size) {
                    counts[index]++
                    budget--
                }
            }
        }
        return ordered.mapIndexed { index, watch -> watch.copy(samples = watch.samples.take(counts[index])) }
    }

    fun speaksAt(watch: Watch, sample: DoubleArray): Boolean {
        if (!watch.home) return false
        val lat1 = Math.toRadians(watch.lat)
        val lat2 = Math.toRadians(sample[0])
        val dLat = lat2 - lat1
        val dLon = Math.toRadians(sample[1] - watch.lon)
        val a = Math.sin(dLat / 2).let { it * it } +
            Math.cos(lat1) * Math.cos(lat2) * Math.sin(dLon / 2).let { it * it }
        return 3958.8 * 2 * Math.atan2(Math.sqrt(a), Math.sqrt(1 - a)) <= 30.01
    }

    /** Destination point `distMi` from (`lat`,`lon`) on `bearingDeg`, spherical earth. */
    private fun offset(lat: Double, lon: Double, distMi: Double, bearingDeg: Double): DoubleArray {
        val ang = distMi / 3958.8
        val br = Math.toRadians(bearingDeg)
        val la = Math.toRadians(lat)
        val lo = Math.toRadians(lon)
        val la2 = Math.asin(Math.sin(la) * Math.cos(ang) + Math.cos(la) * Math.sin(ang) * Math.cos(br))
        val lo2 = lo + Math.atan2(
            Math.sin(br) * Math.sin(ang) * Math.cos(la),
            Math.cos(ang) - Math.sin(la) * Math.sin(la2),
        )
        return doubleArrayOf(Math.toDegrees(la2), Math.toDegrees(lo2))
    }

    fun pointUrl(lat: Double, lon: Double): String =
        "https://api.weather.gov/alerts/active?point=%.4f,%.4f".format(Locale.ROOT, lat, lon)

    fun gotoPoint(lon: Double, lat: Double): String =
        ",%.4f,%.4f,9".format(Locale.ROOT, lon, lat)

    internal fun coverageKey(watched: List<Watch>): String = org.json.JSONArray(watched.map { watch ->
        JSONObject().put("name", watch.name).put("lat", watch.lat).put("lon", watch.lon)
            .put("home", watch.home).put("requested", watch.requestedSamples)
            .put("samples", org.json.JSONArray(watch.samples.map { org.json.JSONArray(it.toList()) }))
    }).toString()

    fun checkSummary(
        hasPlaces: Boolean, lines: List<String>, successful: Int, failed: Int, lastCompleteCheck: Long,
        incomplete: List<String> = emptyList(),
    ): String {
        if (!hasPlaces) return "No saved locations yet"
        val alerts = lines.distinct().take(4).joinToString("\n")
        if (failed == 0 && successful > 0 && incomplete.isEmpty()) return alerts.ifEmpty { "All quiet" }
        val status = if (successful > 0) "Some locations couldn't be checked" else "Couldn't check alerts"
        val coverage = if (incomplete.isEmpty()) "" else {
            "\nPartial coverage: " + incomplete.distinct().take(3).joinToString(", ") +
                if (incomplete.distinct().size > 3) " and ${incomplete.distinct().size - 3} more" else ""
        }
        val previous = if (lastCompleteCheck > 0) {
            "\nLast complete check: " + java.text.DateFormat.getDateTimeInstance(
                java.text.DateFormat.SHORT, java.text.DateFormat.SHORT
            ).format(java.util.Date(lastCompleteCheck))
        } else ""
        return listOf(alerts, status + coverage + previous).filter { it.isNotEmpty() }.joinToString("\n")
    }

    fun alertsAt(lat: Double, lon: Double, deadline: Long, shouldContinue: () -> Boolean): AlertResponse = runCatching {
        val features = fetchFeatures(URL(pointUrl(lat, lon)), deadline, shouldContinue)
        AlertResponse((0 until features.length()).mapNotNull { i ->
            val p = features.optJSONObject(i)?.optJSONObject("properties") ?: return@mapNotNull null
            val event = p.optString("event")
            val id = p.optString("id").ifEmpty { event + p.optString("sent") }
            Alert(id, event, p.optString("headline", event), tierOf(p, event))
        }.filter { it.tier > 0 })
    }.getOrElse { AlertResponse(emptyList(), it.message ?: it.javaClass.simpleName) }

    /** Geometry-backed alerts touching a radius. Point lookup remains the zone-only fallback. */
    fun alertsWithin(lat: Double, lon: Double, radiusMi: Double, deadline: Long, shouldContinue: () -> Boolean): AlertResponse = runCatching {
        val features = fetchFeatures(URL("https://api.weather.gov/alerts/active"), deadline, shouldContinue)
        AlertResponse((0 until features.length()).mapNotNull { i ->
            val feature = features.optJSONObject(i) ?: return@mapNotNull null
            if (!geometryTouches(feature.optJSONObject("geometry"), lat, lon, radiusMi)) {
                return@mapNotNull null
            }
            val p = feature.optJSONObject("properties") ?: return@mapNotNull null
            val event = p.optString("event")
            val id = p.optString("id").ifEmpty { event + p.optString("sent") }
            Alert(id, event, p.optString("headline", event), tierOf(p, event))
        }.filter { it.tier > 0 })
    }.getOrElse { AlertResponse(emptyList(), it.message ?: it.javaClass.simpleName) }

    /** The pass deadline also bounds trickling bodies; per-read timeouts alone do not. */
    internal fun fetchFeatures(url: URL, deadline: Long, shouldContinue: () -> Boolean): org.json.JSONArray {
        fun remaining(): Int {
            check(shouldContinue()) { "Alert check cancelled" }
            val millis = (deadline - System.nanoTime()) / 1_000_000
            check(millis > 0) { "Alert check timed out" }
            return millis.coerceAtMost(5_000).toInt()
        }
        val conn = (url.openConnection() as HttpURLConnection).apply {
            setRequestProperty("User-Agent", USER_AGENT)
            setRequestProperty("Accept", "application/geo+json")
            instanceFollowRedirects = false
            // responseCode may both connect and read headers before control returns.
            val timeout = (remaining() / 2).coerceAtLeast(1)
            connectTimeout = timeout
            readTimeout = timeout
        }
        val body = try {
            if (conn.responseCode != 200) error("NWS HTTP ${conn.responseCode}")
            check(conn.contentLengthLong <= MAX_RESPONSE_BYTES) { "NWS response too large" }
            val output = java.io.ByteArrayOutputStream()
            conn.inputStream.use { input ->
                val buffer = ByteArray(8192)
                while (true) {
                    conn.readTimeout = remaining()
                    val count = input.read(buffer)
                    remaining()
                    if (count < 0) break
                    check(output.size() + count <= MAX_RESPONSE_BYTES) { "NWS response too large" }
                    output.write(buffer, 0, count)
                }
            }
            output.toString(Charsets.UTF_8.name())
        } finally {
            conn.disconnect()
        }
        val features = JSONObject(body).optJSONArray("features") ?: error("NWS response has no alert list")
        remaining()
        return features
    }

    private const val MAX_RESPONSE_BYTES = 32 * 1024 * 1024

    internal fun geometryTouches(geometry: JSONObject?, lat: Double, lon: Double, radiusMi: Double): Boolean {
        geometry ?: return false
        val coordinates = geometry.optJSONArray("coordinates") ?: return false
        fun touchesPolygon(polygon: JSONArray): Boolean {
            val outer = polygon.optJSONArray(0) ?: return false
            return when (ringContact(outer, lat, lon, radiusMi)) {
                RingContact.BOUNDARY -> true
                RingContact.OUTSIDE -> false
                RingContact.INSIDE -> (1 until polygon.length()).none { i ->
                    val hole = polygon.optJSONArray(i) ?: return@none false
                    ringContact(hole, lat, lon, radiusMi) == RingContact.INSIDE
                }
            }
        }
        return when (geometry.optString("type")) {
            "Polygon" -> touchesPolygon(coordinates)
            "MultiPolygon" -> (0 until coordinates.length()).any { i ->
                coordinates.optJSONArray(i)?.let(::touchesPolygon) == true
            }
            else -> false
        }
    }

    private enum class RingContact { OUTSIDE, INSIDE, BOUNDARY }

    private fun ringContact(ring: JSONArray, lat: Double, lon: Double, radiusMi: Double): RingContact {
        val longitudeScale = Math.cos(Math.toRadians(lat)) * 69.172
        var previousOffset = 0.0
        val points = (0 until ring.length()).mapNotNull { i ->
            val p = ring.optJSONArray(i) ?: return@mapNotNull null
            val offset = p.optDouble(0) - lon
            val y = (p.optDouble(1) - lat) * 69.0
            if (!offset.isFinite() || !y.isFinite()) return@mapNotNull null
            previousOffset += Math.IEEEremainder(offset - previousOffset, 360.0)
            doubleArrayOf(previousOffset * longitudeScale, y)
        }
        if (points.size < 3) return RingContact.OUTSIDE
        var inside = false
        var previous = points.last()
        for (point in points) {
            if ((point[1] > 0) != (previous[1] > 0) &&
                0 < (previous[0] - point[0]) * -point[1] / (previous[1] - point[1]) + point[0]
            ) inside = !inside
            val dx = previous[0] - point[0]
            val dy = previous[1] - point[1]
            val length2 = dx * dx + dy * dy
            val t = if (length2 == 0.0) 0.0 else
                (-(point[0] * dx + point[1] * dy) / length2).coerceIn(0.0, 1.0)
            val nearestX = point[0] + t * dx
            val nearestY = point[1] + t * dy
            if (nearestX * nearestX + nearestY * nearestY <= radiusMi * radiusMi) return RingContact.BOUNDARY
            previous = point
        }
        return if (inside) RingContact.INSIDE else RingContact.OUTSIDE
    }

    /** Notification tier used for quiet hours and channel selection. */
    internal fun tierOf(p: JSONObject, event: String): Int {
        val parameters = p.optJSONObject("parameters")
        fun threat(name: String) = parameters?.optJSONArray(name)?.optString(0).orEmpty().uppercase()
        val text = (p.optString("headline") + " " + p.optString("description"))
            .uppercase().replace(Regex("\\s+"), " ")
        val pds = text.contains("PARTICULARLY DANGEROUS SITUATION")
        val escalated = when (event) {
            "Tornado Warning" -> pds || text.contains("TORNADO EMERGENCY") ||
                threat("tornadoDamageThreat") in setOf("CONSIDERABLE", "CATASTROPHIC")
            "Severe Thunderstorm Warning" -> pds || threat("thunderstormDamageThreat") == "DESTRUCTIVE"
            "Flash Flood Warning" -> pds || text.contains("FLASH FLOOD EMERGENCY") ||
                threat("flashFloodDamageThreat") == "CATASTROPHIC"
            "Tornado Emergency", "Flash Flood Emergency" -> true
            else -> false
        }
        return when {
            escalated -> TIER_EMERGENCY
            event.endsWith("Warning") -> TIER_WARNING
            event.endsWith("Watch") -> TIER_WATCH
            else -> 0
        }
    }
}
