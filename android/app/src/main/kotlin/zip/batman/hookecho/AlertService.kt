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
import android.os.PowerManager

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
 * [AlertWorker] is the safety net underneath it, not a replacement — both run the same
 * [pollOnce].
 *
 * [AlertAlarm] is the third leg: an exact alarm is the one schedule Doze honours on time, so a
 * poll still lands when this service has been killed and WorkManager's 15-minute floor is too
 * slow to be a warning.
 */
class AlertService : Service() {
    @Volatile private var running = false

    override fun onBind(intent: Intent?): IBinder? = null

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        if (running) return START_STICKY
        running = true
        createChannels(this)
        startForeground(FOREGROUND_ID, statusNotification())
        Thread { pollLoop() }.start()
        return START_STICKY
    }

    override fun onDestroy() {
        running = false
        super.onDestroy()
    }

    private fun pollLoop() {
        val seen = loadSeen(this)
        while (running) {
            // Wake lock only around the network pass: holding it across the sleep would keep the
            // CPU up for five minutes to do a second of work.
            val wl = getSystemService(PowerManager::class.java)
                .newWakeLock(PowerManager.PARTIAL_WAKE_LOCK, "hookecho:alerts")
            val hot = try {
                wl.acquire(60_000L)
                pollOnce(this, seen)
            } finally {
                if (wl.isHeld) wl.release()
            }
            // The widget's own 30-minute clock is a floor; while the service runs it stays as
            // fresh as the poll it just did.
            AlertWidget.refresh(this)
            AlertAlarm.markPolled(this)
            // Tighten the cadence while something is actually warned at a watched point.
            val waitMs = if (hot) 60_000L else 300_000L
            var slept = 0L
            while (running && slept < waitMs) {
                Thread.sleep(5_000L.coerceAtMost(waitMs - slept))
                slept += 5_000L
            }
        }
    }

    private fun statusNotification(): Notification =
        Notification.Builder(this, CH_STATUS)
            .setSmallIcon(android.R.drawable.ic_menu_compass)
            .setContentTitle("Hook Echo-WX is watching your locations")
            .setContentText("Alerts arrive even with the app closed")
            .setOngoing(true)
            .build()

    companion object {
        private const val FOREGROUND_ID = 1
        private const val CH_STATUS = "status"
        private const val CH_WATCH = "watch"
        private const val CH_WARNING = "warning"
        private const val CH_EMERGENCY = "emergency"
        private const val PREFS = "alerts"
        private const val KEY_SEEN = "seen"
        private const val KEY_ENABLED = "enabled"

        /** Newest-last, so trimming from the front drops the oldest IDs. */
        private const val SEEN_CAP = 200

        private fun prefs(context: Context) =
            context.getSharedPreferences(PREFS, Context.MODE_PRIVATE)

        /** Whether the user's switch is on — survives process death, unlike the service itself. */
        @JvmStatic
        fun isEnabled(context: Context): Boolean = prefs(context).getBoolean(KEY_ENABLED, false)

        /**
         * Already-notified alert IDs. Memory-only dedup re-notified everything the first time the
         * process was recreated, which for a long-lived warning is a duplicate every few minutes.
         */
        fun loadSeen(context: Context): LinkedHashSet<String> =
            LinkedHashSet(prefs(context).getString(KEY_SEEN, "").orEmpty().split("\n").filter { it.isNotEmpty() })

        private fun saveSeen(context: Context, seen: LinkedHashSet<String>) {
            while (seen.size > SEEN_CAP) seen.remove(seen.first())
            prefs(context).edit().putString(KEY_SEEN, seen.joinToString("\n")).apply()
        }

        /**
         * One poll pass over every watched point: notify what is new, return whether anything at
         * warning tier or above is active (the service uses it to tighten its cadence).
         *
         * Shared by the service and [AlertWorker] so there is exactly one poll path.
         */
        @JvmStatic
        fun pollOnce(context: Context, seen: LinkedHashSet<String>): Boolean {
            var hot = false
            val before = seen.size
            // Quiet hours silences everything below the emergency tier. The alert is still
            // recorded as seen, so it does not re-fire the moment the window ends.
            val quiet = Nws.inQuietHours(
                context.filesDir,
                java.util.Calendar.getInstance().get(java.util.Calendar.HOUR_OF_DAY),
            )
            runCatching {
                for (m in Nws.watched(context.filesDir)) {
                    for (p in m.samples) {
                        for (a in Nws.alertsAt(p[0], p[1])) {
                            if (a.tier >= Nws.TIER_WARNING) hot = true
                            val loud = !quiet || a.tier >= Nws.TIER_EMERGENCY
                            if (seen.add(a.id) && loud) notify(context, m, a)
                        }
                    }
                }
            }.onFailure { /* offline or NWS hiccup: try again next pass */ }
            if (seen.size != before) saveSeen(context, seen)
            return hot
        }

        private fun notify(context: Context, m: Nws.Watch, a: Nws.Alert) {
            // Deep-link back to the watched point. The site field is left empty: the app keeps
            // whichever radar it was on and just flies the camera there.
            val goto = ",%.4f,%.4f,9".format(m.lon, m.lat)
            val intent = Intent(context, MainActivity::class.java)
                .setFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_SINGLE_TOP)
                .putExtra(MainActivity.EXTRA_GOTO, goto)
            val pending = PendingIntent.getActivity(
                context, a.id.hashCode(), intent,
                PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE,
            )
            val channel = when (a.tier) {
                Nws.TIER_EMERGENCY -> CH_EMERGENCY
                Nws.TIER_WARNING -> CH_WARNING
                else -> CH_WATCH
            }
            val n = Notification.Builder(context, channel)
                .setSmallIcon(android.R.drawable.ic_dialog_alert)
                .setContentTitle("${a.event} — ${m.name}")
                .setContentText(a.headline)
                .setStyle(Notification.BigTextStyle().bigText(a.headline))
                .setContentIntent(pending)
                .setAutoCancel(true)
                .build()
            manager(context).notify(a.id.hashCode(), n)
        }

        private fun manager(context: Context) = context.getSystemService(NotificationManager::class.java)

        /** One channel per tier so the user can silence watches without silencing warnings. */
        @JvmStatic
        fun createChannels(context: Context) {
            val m = manager(context)
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

        /**
         * Start/stop entry point, called from Rust over JNI (see `platform::set_background_alerts`)
         * so the in-app switch is the only control surface.
         */
        @JvmStatic
        fun setEnabled(context: Context, enabled: Boolean) {
            prefs(context).edit().putBoolean(KEY_ENABLED, enabled).apply()
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
                AlertWorker.enqueue(context)
                AlertAlarm.arm(context)
            } else {
                context.stopService(intent)
                AlertWorker.cancel(context)
                AlertAlarm.cancel(context)
            }
        }
    }
}
