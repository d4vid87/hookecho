package io.hookecho.HookEcho

import android.app.AlarmManager
import android.app.PendingIntent
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.net.Uri
import android.os.Build
import android.os.PowerManager

/**
 * The third leg under background alerting, and the only one the OS cannot defer.
 *
 * [AlertService] is a foreground service, which Samsung's app-sleep and deep Doze both kill.
 * [AlertWorker] survives that, but WorkManager's periodic floor is 15 minutes and it is a floor,
 * not a promise — a tornado warning that arrives 15 minutes late is not a warning. An exact alarm
 * (`setExactAndAllowWhileIdle`) is the one schedule Doze honours, so a poll runs on time with the
 * app closed, the service dead and the screen off.
 *
 * Exact alarms are permissioned from API 31. We ask for `SCHEDULE_EXACT_ALARM` (the
 * user-grantable one, prompted through its settings screen) rather than `USE_EXACT_ALARM` (the
 * one reserved for alarm-clock and calendar apps, which a weather app would not survive review
 * claiming). When it is not granted we fall back to `setAndAllowWhileIdle`, which still wakes in
 * Doze but on the OS's own schedule — better than nothing, and honestly reported in the health
 * readout instead of quietly pretending.
 *
 * ponytail: one alarm at a fixed 5-minute cadence, re-armed by each pass. Cadence that tightens
 * while a warning is active would mean cancelling and re-arming on every poll; the service
 * already does the tightening when it is alive, and this is the path for when it isn't.
 */
class AlertAlarm : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
        if (!AlertService.isEnabled(context)) return
        AlertService.createChannels(context)
        // The alarm wakes the CPU only long enough to deliver this broadcast; a poll is a network
        // round trip, so it needs its own lock or the device sleeps mid-fetch.
        val wl = context.getSystemService(PowerManager::class.java)
            .newWakeLock(PowerManager.PARTIAL_WAKE_LOCK, "hookecho:alarm")
        try {
            wl.acquire(60_000L)
            AlertService.pollOnce(context, AlertService.loadSeen(context))
            AlertWidget.refresh(context)
            markPolled(context)
        } finally {
            if (wl.isHeld) wl.release()
            // Re-arm last and unconditionally: a poll that threw must not end the chain.
            arm(context)
        }
    }

    companion object {
        /** How often the alarm polls. Matches the service's calm-weather cadence. */
        private const val PERIOD_MS = 5 * 60 * 1000L

        /** …and under battery saver, WorkManager's own floor: three wakeups an hour, not twelve. */
        private const val PERIOD_SAVER_MS = 15 * 60 * 1000L
        private const val PREFS = "alerts"
        private const val KEY_LAST_POLL = "last_alarm_poll"
        private const val KEY_NEXT = "next_alarm"

        private fun prefs(context: Context) =
            context.getSharedPreferences(PREFS, Context.MODE_PRIVATE)

        private fun intent(context: Context) = PendingIntent.getBroadcast(
            context, 0, Intent(context, AlertAlarm::class.java),
            PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE,
        )

        private fun alarms(context: Context) = context.getSystemService(AlarmManager::class.java)

        /** Whether the OS will honour an exact alarm from us right now. */
        @JvmStatic
        fun canExact(context: Context): Boolean =
            Build.VERSION.SDK_INT < 31 || alarms(context).canScheduleExactAlarms()

        /** Whether we are exempt from battery optimisation (Doze's app-standby buckets). */
        @JvmStatic
        fun isExempt(context: Context): Boolean =
            context.getSystemService(PowerManager::class.java)
                .isIgnoringBatteryOptimizations(context.packageName)

        /**
         * Schedule the next poll. Exact where permitted, `setAndAllowWhileIdle` where not — both
         * fire in Doze, only one fires on time.
         */
        @JvmStatic
        fun arm(context: Context) {
            if (!AlertService.isEnabled(context)) return
            val period = if (AlertService.batterySaver(context)) PERIOD_SAVER_MS else PERIOD_MS
            val at = System.currentTimeMillis() + period
            val pi = intent(context)
            // A denied exact alarm throws rather than degrading, so the fallback is a catch too:
            // the permission can be revoked between the check and the call.
            runCatching {
                if (canExact(context)) {
                    alarms(context).setExactAndAllowWhileIdle(AlarmManager.RTC_WAKEUP, at, pi)
                } else {
                    alarms(context).setAndAllowWhileIdle(AlarmManager.RTC_WAKEUP, at, pi)
                }
            }.onFailure {
                alarms(context).setAndAllowWhileIdle(AlarmManager.RTC_WAKEUP, at, pi)
            }
            prefs(context).edit().putLong(KEY_NEXT, at).apply()
        }

        @JvmStatic
        fun cancel(context: Context) {
            alarms(context).cancel(intent(context))
            prefs(context).edit().remove(KEY_NEXT).apply()
        }

        /** Record that a poll actually happened, for the health readout. */
        @JvmStatic
        fun markPolled(context: Context) {
            prefs(context).edit().putLong(KEY_LAST_POLL, System.currentTimeMillis()).apply()
        }

        /**
         * Open the OS screen that grants exact alarms. No-op below API 31, where the permission
         * does not exist and alarms are already exact.
         */
        @JvmStatic
        fun requestExact(context: Context) {
            if (Build.VERSION.SDK_INT < 31 || canExact(context)) return
            runCatching {
                context.startActivity(
                    Intent(android.provider.Settings.ACTION_REQUEST_SCHEDULE_EXACT_ALARM)
                        .setData(Uri.parse("package:${context.packageName}"))
                        .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
                )
            }
        }

        /**
         * Ask to be exempt from battery optimisation. This is the dialog that actually decides
         * whether alerts arrive overnight on a Samsung.
         *
         * `REQUEST_IGNORE_BATTERY_OPTIMIZATIONS` is Play-policy-sensitive — the store allows it
         * for apps whose core function needs it and rejects it otherwise. This build is sideloaded
         * from GitHub releases, where the prompt is fine; whether a Play submission keeps it is a
         * decision for whoever files that submission, and it is one call to delete.
         */
        @JvmStatic
        fun requestExemption(context: Context) {
            if (isExempt(context)) return
            runCatching {
                context.startActivity(
                    Intent(android.provider.Settings.ACTION_REQUEST_IGNORE_BATTERY_OPTIMIZATIONS)
                        .setData(Uri.parse("package:${context.packageName}"))
                        .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
                )
            }
        }

        /**
         * Delivery health, as a tab-separated line for the Rust settings panel to parse:
         * `enabled \t exact \t exempt \t lastPollMsAgo \t nextAlarmInMs`. -1 means never/unknown.
         *
         * Deliberately one string over the existing bridge rather than five JNI calls or a second
         * bridge: the panel reads it once a second at most.
         */
        @JvmStatic
        fun health(context: Context): String {
            val p = prefs(context)
            val now = System.currentTimeMillis()
            val last = p.getLong(KEY_LAST_POLL, 0L)
            val next = p.getLong(KEY_NEXT, 0L)
            return listOf(
                AlertService.isEnabled(context),
                canExact(context),
                isExempt(context),
                if (last > 0) now - last else -1L,
                if (next > 0) next - now else -1L,
            ).joinToString("\t")
        }
    }
}
