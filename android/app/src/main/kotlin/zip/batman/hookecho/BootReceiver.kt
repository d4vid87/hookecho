package zip.batman.hookecho

import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent

/**
 * Re-arms background alerting after a reboot.
 *
 * Enqueues the worker and nothing else: Android 15 (targetSdk 35) forbids starting a `dataSync`
 * foreground service from BOOT_COMPLETED, so the real service comes back on the next app open —
 * until then [AlertWorker] carries the polls.
 */
class BootReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
        if (intent.action != Intent.ACTION_BOOT_COMPLETED) return
        if (!AlertService.isEnabled(context)) return
        AlertWorker.enqueue(context)
        // Alarms do not survive a reboot; this is the only thing that re-arms them.
        AlertAlarm.arm(context)
    }
}
