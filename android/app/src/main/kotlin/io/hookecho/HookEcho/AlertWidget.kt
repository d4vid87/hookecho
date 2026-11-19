package io.hookecho.HookEcho

import android.app.PendingIntent
import android.appwidget.AppWidgetManager
import android.appwidget.AppWidgetProvider
import android.content.ComponentName
import android.content.Context
import android.content.Intent
import android.widget.RemoteViews
import java.io.File

/**
 * Home-screen widget: what is currently warned at your saved locations, and a tap that opens the
 * app on the first one.
 *
 * Poll results come from the same pass used by the service. A widget can request a pass with
 * background notifications switched off, without keeping its broadcast alive for network I/O.
 */
class AlertWidget : AppWidgetProvider() {
    override fun onUpdate(context: Context, manager: AppWidgetManager, ids: IntArray) {
        updateViews(context, manager, ids, "Checking saved locations…", null)
        AlertWorker.enqueueNow(context)
    }

    companion object {
        internal fun hasWidgets(context: Context): Boolean = AppWidgetManager.getInstance(context)
            .getAppWidgetIds(ComponentName(context, AlertWidget::class.java)).isNotEmpty()

        internal fun render(context: Context, result: AlertPoller.Result) {
            val manager = AppWidgetManager.getInstance(context)
            val ids = manager.getAppWidgetIds(ComponentName(context, AlertWidget::class.java))
            if (ids.isEmpty()) return
            val watched = result.watched
            val coverage = Nws.coverageKey(watched)
            val prefs = context.getSharedPreferences("alert-widget", Context.MODE_PRIVATE)
            var lastComplete = if (prefs.getString("coverage", null) == coverage) prefs.getLong("complete_at", 0) else 0
            if (result.complete) {
                lastComplete = System.currentTimeMillis()
                prefs.edit().putString("coverage", coverage).putLong("complete_at", lastComplete).apply()
            }
            var text = result.error ?: Nws.checkSummary(watched.isNotEmpty(), result.lines,
                result.successfulRequests, result.failedRequests, lastComplete, result.incomplete)
            // The storm line the app last wrote for the radar widget, reused here: "all quiet" and
            // "a cell 12 miles away" are different kinds of quiet, and this widget has no cell data
            // of its own. Absent when the app has not run, which is simply no line.
            runCatching {
                File(context.filesDir, RadarWidget.CAPTION).takeIf { it.exists() }?.readText()?.trim()
            }.getOrNull()?.takeIf { it.isNotEmpty() }?.let { text = "$text\n$it" }
            updateViews(context, manager, ids, text, watched.firstOrNull())
        }

        private fun updateViews(
            context: Context, manager: AppWidgetManager, ids: IntArray, text: String, watch: Nws.Watch?,
        ) {
            // Tapping opens the app at the first watched point (site left empty: keep the current
            // radar, just fly there), same deep-link string the notifications use.
            val intent = Intent(context, MainActivity::class.java)
                .setFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_SINGLE_TOP)
            watch?.let {
                intent.putExtra(MainActivity.EXTRA_GOTO, Nws.gotoPoint(it.lon, it.lat))
            }
            val tap = PendingIntent.getActivity(
                context, 0, intent,
                PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE,
            )
            val views = RemoteViews(context.packageName, R.layout.widget_alerts).apply {
                setTextViewText(R.id.widget_alerts_text, text)
                setOnClickPendingIntent(R.id.widget_alerts_root, tap)
            }
            for (id in ids) manager.updateAppWidget(id, views)
        }
    }
}
