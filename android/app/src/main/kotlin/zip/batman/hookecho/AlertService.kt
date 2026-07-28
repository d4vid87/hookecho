package zip.batman.hookecho

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.content.Context
import android.content.Intent
import android.os.Build
import android.os.IBinder
import org.json.JSONObject
import java.io.File
import java.net.HttpURLConnection
import java.net.URL

/**
 * Watches the user's saved markers for NWS alerts while the app is closed.
 *
 * Deliberately Kotlin-only, with no bridge back into Rust: `api.weather.gov/alerts/active?point=`
 * returns every alert covering a point with no geometry work, so the whole watcher is a fetch, a
 * JSON walk and a notification. The watch list is read from the same `settings.json` the app
 * writes (`markers[].name/lat/lon`, plus the `background_alerts` switch) — a shared file instead
 * of an IPC channel, since the service usually runs when the app does not.
 *
 * A foreground service, not WorkManager: a tornado warning that arrives on the OS's deferred
 * schedule is not a warning. The cost is the permanent notification, which is why this is opt-in.
 */
class AlertService : Service() {
    @Volatile private var running = false
    private val seen = HashSet<String>()

    override fun onBind(intent: Intent?): IBinder? = null

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        if (running) return START_STICKY
        running = true
        createChannels()
        startForeground(FOREGROUND_ID, statusNotification())
        Thread { pollLoop() }.start()
        return START_STICKY
    }

    override fun onDestroy() {
        running = false
        super.onDestroy()
    }

    private fun pollLoop() {
        while (running) {
            var hot = false
            runCatching {
                for (m in watched()) {
                    for (a in alertsAt(m.lat, m.lon)) {
                        if (a.tier >= TIER_WARNING) hot = true
                        if (seen.add(a.id)) notify(m, a)
                    }
                }
            }.onFailure { /* offline or NWS hiccup: try again next pass */ }
            // Tighten the cadence while something is actually warned at a watched point.
            val waitMs = if (hot) 60_000L else 300_000L
            var slept = 0L
            while (running && slept < waitMs) {
                Thread.sleep(5_000L.coerceAtMost(waitMs - slept))
                slept += 5_000L
            }
        }
    }

    private data class Watch(val name: String, val lat: Double, val lon: Double)

    private data class Alert(val id: String, val event: String, val headline: String, val tier: Int)

    /** Saved markers from settings.json — the same file the Rust app reads and writes. */
    private fun watched(): List<Watch> {
        val f = File(filesDir, "config/settings.json")
        if (!f.exists()) return emptyList()
        val markers = JSONObject(f.readText()).optJSONArray("markers") ?: return emptyList()
        return (0 until markers.length()).mapNotNull { i ->
            val m = markers.optJSONObject(i) ?: return@mapNotNull null
            Watch(m.optString("name", "Saved location"), m.optDouble("lat"), m.optDouble("lon"))
        }.filter { it.lat.isFinite() && it.lon.isFinite() }
    }

    private fun alertsAt(lat: Double, lon: Double): List<Alert> {
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

    private fun notify(m: Watch, a: Alert) {
        // Deep-link back to the watched point. The site field is left empty: the app keeps
        // whichever radar it was on and just flies the camera there.
        val goto = ",%.4f,%.4f,9".format(m.lon, m.lat)
        val intent = Intent(this, MainActivity::class.java)
            .setFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_SINGLE_TOP)
            .putExtra(MainActivity.EXTRA_GOTO, goto)
        val pending = PendingIntent.getActivity(
            this, a.id.hashCode(), intent,
            PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE,
        )
        val channel = when (a.tier) {
            TIER_EMERGENCY -> CH_EMERGENCY
            TIER_WARNING -> CH_WARNING
            else -> CH_WATCH
        }
        val n = Notification.Builder(this, channel)
            .setSmallIcon(android.R.drawable.ic_dialog_alert)
            .setContentTitle("${a.event} — ${m.name}")
            .setContentText(a.headline)
            .setStyle(Notification.BigTextStyle().bigText(a.headline))
            .setContentIntent(pending)
            .setAutoCancel(true)
            .build()
        manager().notify(a.id.hashCode(), n)
    }

    private fun statusNotification(): Notification =
        Notification.Builder(this, CH_STATUS)
            .setSmallIcon(android.R.drawable.ic_menu_compass)
            .setContentTitle("Hook Echo-WX is watching your locations")
            .setContentText("Alerts arrive even with the app closed")
            .setOngoing(true)
            .build()

    private fun manager() = getSystemService(NotificationManager::class.java)

    /** One channel per tier so the user can silence watches without silencing warnings. */
    private fun createChannels() {
        val m = manager()
        m.createNotificationChannel(
            NotificationChannel(CH_STATUS, "Background watch", NotificationManager.IMPORTANCE_LOW)
        )
        m.createNotificationChannel(
            NotificationChannel(CH_WATCH, "Watches", NotificationManager.IMPORTANCE_DEFAULT)
        )
        m.createNotificationChannel(
            NotificationChannel(CH_WARNING, "Warnings", NotificationManager.IMPORTANCE_HIGH)
        )
        m.createNotificationChannel(
            NotificationChannel(CH_EMERGENCY, "Emergencies", NotificationManager.IMPORTANCE_HIGH)
                .apply { enableVibration(true); setBypassDnd(true) }
        )
    }

    companion object {
        private const val FOREGROUND_ID = 1
        private const val CH_STATUS = "status"
        private const val CH_WATCH = "watch"
        private const val CH_WARNING = "warning"
        private const val CH_EMERGENCY = "emergency"
        private const val TIER_WATCH = 1
        private const val TIER_WARNING = 2
        private const val TIER_EMERGENCY = 3
        private const val USER_AGENT = "hookecho (github.com/d4vid87/hookecho)"

        /**
         * Start/stop entry point, called from Rust over JNI (see `platform::set_background_alerts`)
         * so the in-app switch is the only control surface.
         */
        @JvmStatic
        fun setEnabled(context: Context, enabled: Boolean) {
            val intent = Intent(context, AlertService::class.java)
            if (enabled) {
                // API 33+ won't show a thing without the runtime grant, and the moment the user
                // flips the switch is the only moment the prompt makes sense.
                if (Build.VERSION.SDK_INT >= 33 && context is android.app.Activity &&
                    context.checkSelfPermission(android.Manifest.permission.POST_NOTIFICATIONS) !=
                    android.content.pm.PackageManager.PERMISSION_GRANTED
                ) {
                    context.requestPermissions(
                        arrayOf(android.Manifest.permission.POST_NOTIFICATIONS), 1
                    )
                }
                context.startForegroundService(intent)
            } else {
                context.stopService(intent)
            }
        }
    }
}
