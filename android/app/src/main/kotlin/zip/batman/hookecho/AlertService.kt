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
                for (m in Nws.watched(filesDir)) {
                    for (a in Nws.alertsAt(m.lat, m.lon)) {
                        if (a.tier >= Nws.TIER_WARNING) hot = true
                        if (seen.add(a.id)) notify(m, a)
                    }
                }
            }.onFailure { /* offline or NWS hiccup: try again next pass */ }
            // The widget's own 30-minute clock is a floor; while the service runs it stays as
            // fresh as the poll it just did.
            AlertWidget.refresh(this)
            // Tighten the cadence while something is actually warned at a watched point.
            val waitMs = if (hot) 60_000L else 300_000L
            var slept = 0L
            while (running && slept < waitMs) {
                Thread.sleep(5_000L.coerceAtMost(waitMs - slept))
                slept += 5_000L
            }
        }
    }

    private fun notify(m: Nws.Watch, a: Nws.Alert) {
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
            Nws.TIER_EMERGENCY -> CH_EMERGENCY
            Nws.TIER_WARNING -> CH_WARNING
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
