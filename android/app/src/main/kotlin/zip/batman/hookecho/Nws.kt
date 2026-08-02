package zip.batman.hookecho

import org.json.JSONObject
import java.io.File
import java.net.HttpURLConnection
import java.net.URL

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

    data class Watch(val name: String, val lat: Double, val lon: Double)

    data class Alert(val id: String, val event: String, val headline: String, val tier: Int)

    /** Saved markers from settings.json — the same file the Rust app reads and writes. */
    fun watched(filesDir: File): List<Watch> {
        val f = File(filesDir, "config/settings.json")
        if (!f.exists()) return emptyList()
        val markers = JSONObject(f.readText()).optJSONArray("markers") ?: return emptyList()
        return (0 until markers.length()).mapNotNull { i ->
            val m = markers.optJSONObject(i) ?: return@mapNotNull null
            Watch(m.optString("name", "Saved location"), m.optDouble("lat"), m.optDouble("lon"))
        }.filter { it.lat.isFinite() && it.lon.isFinite() }
    }

    fun alertsAt(lat: Double, lon: Double): List<Alert> {
        val url = URL("https://api.weather.gov/alerts/active?point=%.4f,%.4f".format(lat, lon))
        val conn = (url.openConnection() as HttpURLConnection).apply {
            setRequestProperty("User-Agent", USER_AGENT)
            setRequestProperty("Accept", "application/geo+json")
            connectTimeout = 15_000
            readTimeout = 15_000
        }
        val body = try {
            if (conn.responseCode != 200) return emptyList()
            conn.inputStream.bufferedReader().readText()
        } finally {
            conn.disconnect()
        }
        val features = JSONObject(body).optJSONArray("features") ?: return emptyList()
        return (0 until features.length()).mapNotNull { i ->
            val p = features.optJSONObject(i)?.optJSONObject("properties") ?: return@mapNotNull null
            val event = p.optString("event")
            val id = p.optString("id").ifEmpty { event + p.optString("sent") }
            Alert(id, event, p.optString("headline", event), tierOf(p, event))
        }.filter { it.tier > 0 }
    }

    /**
     * Escalation tier from the alert's own words. `tornadoDamageThreat` is the field that
     * separates a routine tornado warning from a PDS/emergency one, and it is only ever set on
     * the alerts that matter.
     */
    private fun tierOf(p: JSONObject, event: String): Int {
        val threat = p.optJSONObject("parameters")?.optJSONArray("tornadoDamageThreat")
            ?.optString(0).orEmpty()
        val text = (p.optString("description") + " " + p.optString("headline")).uppercase()
        return when {
            threat == "CATASTROPHIC" || text.contains("TORNADO EMERGENCY") -> TIER_EMERGENCY
            threat == "CONSIDERABLE" || event.contains("Flash Flood Emergency") -> TIER_EMERGENCY
            event.endsWith("Warning") -> TIER_WARNING
            event.endsWith("Watch") -> TIER_WATCH
            else -> 0
        }
    }
}
