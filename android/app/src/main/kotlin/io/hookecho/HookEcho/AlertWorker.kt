package io.hookecho.HookEcho

import android.app.ForegroundServiceStartNotAllowedException
import android.content.Context
import android.content.Intent
import android.os.Build
import androidx.work.Constraints
import androidx.work.ExistingPeriodicWorkPolicy
import androidx.work.ExistingWorkPolicy
import androidx.work.NetworkType
import androidx.work.OneTimeWorkRequestBuilder
import androidx.work.PeriodicWorkRequestBuilder
import androidx.work.WorkManager
import androidx.work.Worker
import androidx.work.WorkerParameters
import java.util.concurrent.TimeUnit

/**
 * The safety net under [AlertService]: WorkManager survives process death and reboot, the
 * foreground service does not.
 *
 * It cannot replace the service — 15 minutes is WorkManager's periodic floor, and a warning that
 * arrives 15 minutes late is not a warning. So each pass first tries to get the real service back
 * on its feet, then polls itself regardless, which is the only alert the user gets if the start
 * is refused.
 */
class AlertWorker(context: Context, params: WorkerParameters) : Worker(context, params) {
    override fun doWork(): Result {
        val context = applicationContext
        val enabled = AlertService.isEnabled(context)
        val widget = AlertWidget.hasWidgets(context)
        if (!enabled && !widget) return Result.success()
        if (enabled) AlertService.createChannels(context)
        // Background FGS starts are refused from API 31 unless an exemption applies; when they
        // are, this pass is the alert path.
        try {
            if (enabled) context.startForegroundService(Intent(context, AlertService::class.java))
        } catch (e: Exception) {
            if (Build.VERSION.SDK_INT >= 31 && e !is ForegroundServiceStartNotAllowedException) throw e
        }
        val poll = AlertService.pollOnce(context) {
            !isStopped && (AlertService.isEnabled(context) || widget)
        } ?: return Result.success()
        if (isStopped || poll.cancelled) return Result.success()
        if (enabled && poll.successfulRequests > 0) AlertAlarm.markPolled(context)
        // Watchdog half: an exact alarm is a one-shot that re-arms itself, so anything that eats
        // one — a force-stop, a reboot before BootReceiver, the OS cancelling on package replace
        // — ends the chain silently. Re-arming here every 15 minutes is what restarts it.
        AlertAlarm.arm(context)
        return if (poll.successfulRequests > 0 || (poll.watched.isEmpty() && poll.error == null)) {
            Result.success()
        } else {
            Result.retry()
        }
    }

    companion object {
        private const val NAME = "alert-poll"
        private const val NOW = "alert-poll-now"
        private const val PERMISSION_GRANTED = "alert-poll-permission-granted"

        /** Receiver work is unique and returns immediately; concurrent service polls are shared. */
        fun enqueueNow(context: Context, permissionGranted: Boolean = false) {
            WorkManager.getInstance(context).enqueueUniqueWork(
                if (permissionGranted) PERMISSION_GRANTED else NOW,
                if (permissionGranted) ExistingWorkPolicy.REPLACE else ExistingWorkPolicy.KEEP,
                OneTimeWorkRequestBuilder<AlertWorker>()
                    .setConstraints(Constraints.Builder().setRequiredNetworkType(NetworkType.CONNECTED).build())
                    .build(),
            )
        }

        @JvmStatic
        fun enqueue(context: Context) {
            val work = PeriodicWorkRequestBuilder<AlertWorker>(15, TimeUnit.MINUTES)
                .setConstraints(
                    Constraints.Builder().setRequiredNetworkType(NetworkType.CONNECTED).build()
                )
                .build()
            WorkManager.getInstance(context).enqueueUniquePeriodicWork(
                NAME, ExistingPeriodicWorkPolicy.KEEP, work
            )
        }

        @JvmStatic
        fun cancel(context: Context) {
            WorkManager.getInstance(context).cancelUniqueWork(NAME)
            WorkManager.getInstance(context).cancelUniqueWork(NOW)
            WorkManager.getInstance(context).cancelUniqueWork(PERMISSION_GRANTED)
        }
    }
}
