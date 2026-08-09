package zip.batman.hookecho

import android.app.ForegroundServiceStartNotAllowedException
import android.content.Context
import android.content.Intent
import android.os.Build
import androidx.work.Constraints
import androidx.work.ExistingPeriodicWorkPolicy
import androidx.work.NetworkType
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
        if (!AlertService.isEnabled(context)) return Result.success()
        AlertService.createChannels(context)
        // Background FGS starts are refused from API 31 unless an exemption applies; when they
        // are, this pass is the alert path.
        try {
            context.startForegroundService(Intent(context, AlertService::class.java))
        } catch (e: Exception) {
            if (Build.VERSION.SDK_INT >= 31 && e !is ForegroundServiceStartNotAllowedException) throw e
        }
        AlertService.pollOnce(context, AlertService.loadSeen(context))
        AlertWidget.refresh(context)
        return Result.success()
    }

    companion object {
        private const val NAME = "alert-poll"

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
        }
    }
}
