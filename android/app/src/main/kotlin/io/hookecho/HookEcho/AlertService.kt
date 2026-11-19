package io.hookecho.HookEcho

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
 * [AlertAlarm] is an additional recovery path when this service is stopped. Android can defer
 * both alarms and WorkManager in idle modes; neither promises a warning-delivery deadline.
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

    override fun onTimeout(startId: Int, fgsType: Int) {
        // Android 15 limits dataSync foreground-service time while the app is backgrounded.
        // Stop promptly when its allowance expires; leaving the service alive crashes the app.
        // The user's enabled setting and the worker/alarm recovery schedules remain intact.
        running = false
        stopSelf()
    }

    private fun pollLoop() {
        while (running) {
            // Wake lock only around the network pass: holding it across the sleep would keep the
            // CPU up for five minutes to do a second of work.
            val wl = getSystemService(PowerManager::class.java)
                .newWakeLock(PowerManager.PARTIAL_WAKE_LOCK, "hookecho:alerts")
            val result = try {
                wl.acquire(60_000L)
                pollOnce(this) { running && isEnabled(this) }
            } finally {
                if (wl.isHeld) wl.release()
            }
            if (!running || result == null) break
            if (result.successfulRequests > 0) AlertAlarm.markPolled(this)
            // Tighten the cadence while something is actually warned at a watched point. Battery
            // saver stretches the calm case only: once a warning is up, the user has bigger
            // problems than battery, and the tight loop is the whole reason this service exists.
            val waitMs = when {
                result.hot -> 60_000L
                batterySaver(this) -> 900_000L
                else -> 300_000L
            }
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
            .setContentTitle("HookEcho is watching your locations")
            .setContentText("Alerts arrive even with the app closed")
            .setOngoing(true)
            .build()

    companion object {
        private const val FOREGROUND_ID = 1
        private const val CH_STATUS = "status"
        private const val CH_WATCH = "watch"
        // Versioned id: a channel's vibration is fixed at creation and the OS ignores later
        // edits, so correcting it means a new channel. The old one is deleted in
        // [createChannels] rather than left behind as a second "Warnings" row in settings.
        private const val CH_WARNING = "warning_v2"
        private const val CH_WARNING_OLD = "warning"
        private const val CH_EMERGENCY = "emergency"
        private const val PREFS = "alerts"
        private const val KEY_SEEN = "seen"
        private const val KEY_ENABLED = "enabled"
        private const val KEY_SAVER = "battery_saver"

        /** Newest-last, so trimming from the front drops the oldest IDs. */
        private const val SEEN_CAP = 200

        private val poller = AlertPoller()

        private fun prefs(context: Context) =
            context.getSharedPreferences(PREFS, Context.MODE_PRIVATE)

        /** Whether the user's switch is on — survives process death, unlike the service itself. */
        @JvmStatic
        fun isEnabled(context: Context): Boolean = prefs(context).getBoolean(KEY_ENABLED, false)

        /**
         * Already-notified alert IDs. Memory-only dedup re-notified everything the first time the
         * process was recreated, which for a long-lived warning is a duplicate every few minutes.
         */
        private fun loadSeen(context: Context): LinkedHashSet<String> =
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
        internal fun pollOnce(
            context: Context,
            shouldContinue: () -> Boolean = { true },
        ): AlertPoller.Result? {
            val watched = runCatching { Nws.watched(context.filesDir) }.getOrElse {
                return AlertPoller.Result(failedRequests = 1, error = "Couldn't read saved locations")
                    .also { AlertWidget.render(context, it) }
            }
            val enabled = isEnabled(context)
            val quiet = Nws.inQuietHours(
                context.filesDir,
                java.util.Calendar.getInstance().get(java.util.Calendar.HOUR_OF_DAY),
            )
            val key = "$enabled:$quiet:" + Nws.coverageKey(watched)
            val result = poller.run(key, shouldContinue) {
                // Loading, notification decisions and persistence are one coordinated pass.
                val seen = loadSeen(context)
                val before = seen.size
                try {
                    AlertPoller.collect(
                        watched, System.nanoTime() + AlertPoller.BUDGET_NANOS, shouldContinue,
                        onAlert = { watch, alert, speak ->
                            if (enabled && isEnabled(context) && shouldContinue()) {
                                val loud = !quiet || alert.tier >= Nws.TIER_EMERGENCY
                                if (seen.add(alert.id) && loud) notify(context, watch, alert, speak)
                            }
                        },
                    )
                } finally {
                    if (seen.size != before) saveSeen(context, seen)
                }
            }
            if (result != null && !result.cancelled) {
                AlertWidget.render(context, result)
            }
            return result
        }

        private fun notify(context: Context, m: Nws.Watch, a: Nws.Alert, speak: Boolean) {
            // Deep-link back to the watched point. The site field is left empty: the app keeps
            // whichever radar it was on and just flies the camera there.
            val goto = Nws.gotoPoint(m.lon, m.lat)
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
            if (speak && !MainActivity.foreground && Nws.speechEnabled(context.filesDir)) {
                PiperVoice.enqueue(
                    context,
                    "${a.event} for ${m.name}. ${a.headline}",
                    a.tier,
                )
            }
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
            m.deleteNotificationChannel(CH_WARNING_OLD)
            m.createNotificationChannel(
                NotificationChannel(CH_WARNING, "Warnings", NotificationManager.IMPORTANCE_HIGH)
                    // IMPORTANCE_HIGH does not imply vibration, which is how a tornado warning
                    // arrived silent-to-the-wrist: `mVibrationEnabled=false` on the device. This
                    // is also the whole Wear story — a bridged notification buzzes the watch on
                    // the phone channel's terms, so the watch needed nothing of its own.
                    .apply { enableVibration(true) }
            )
            m.createNotificationChannel(
                NotificationChannel(CH_EMERGENCY, "Emergencies", NotificationManager.IMPORTANCE_HIGH)
                    .apply { enableVibration(true); setBypassDnd(true) }
            )
        }

        /** Whether the user asked for the relaxed polling schedule (see `Settings::battery_saver`). */
        @JvmStatic
        fun batterySaver(context: Context): Boolean = prefs(context).getBoolean(KEY_SAVER, false)

        /**
         * Battery saver, pushed from the in-app switch over JNI. Re-arms the alarm so the new
         * cadence starts now rather than after one more poll on the old one.
         */
        @JvmStatic
        fun setBatterySaver(context: Context, on: Boolean) {
            prefs(context).edit().putBoolean(KEY_SAVER, on).apply()
            if (isEnabled(context)) AlertAlarm.arm(context)
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
