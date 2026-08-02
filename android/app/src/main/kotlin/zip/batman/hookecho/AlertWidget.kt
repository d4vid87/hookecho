package zip.batman.hookecho

import android.app.PendingIntent
import android.appwidget.AppWidgetManager
import android.appwidget.AppWidgetProvider
import android.content.ComponentName
import android.content.Context
import android.content.Intent
import android.widget.RemoteViews

/**
 * Home-screen widget: what is currently warned at your saved locations, and a tap that opens the
 * app on the first one.
 *
 * Pure Kotlin — the widget cannot host the Rust renderer, so it shows text rather than a radar
 * snapshot. It reuses [Nws], which means it works with the background service switched off; the
 * service just pushes an extra refresh whenever it polls.
 */
class AlertWidget : AppWidgetProvider() {
    override fun onUpdate(context: Context, manager: AppWidgetManager, ids: IntArray) {
        // Network on the main thread would crash; goAsync keeps the broadcast alive meanwhile.
        val pending = goAsync()
        Thread {
            runCatching { render(context, manager, ids) }
            pending.finish()
        }.start()
    }

    private fun render(context: Context, manager: AppWidgetManager, ids: IntArray) {
        val watched = Nws.watched(context.filesDir)
        val lines = ArrayList<String>()
        for (m in watched) {
            for (a in Nws.alertsAt(m.lat, m.lon)) {
                lines.add("${a.event} — ${m.name}")
            }
        }
        val text = when {
            watched.isEmpty() -> "No saved locations yet"
            lines.isEmpty() -> "All quiet"
            else -> lines.distinct().take(4).joinToString("\n")
        }
        // Tapping opens the app at the first watched point (site left empty: keep the current
        // radar, just fly there), same deep-link string the notifications use.
        val intent = Intent(context, MainActivity::class.java)
            .setFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_SINGLE_TOP)
        watched.firstOrNull()?.let {
            intent.putExtra(MainActivity.EXTRA_GOTO, ",%.4f,%.4f,9".format(it.lon, it.lat))
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

    companion object {
        /** Re-render every placed widget now (called by the service after each poll). */
        @JvmStatic
        fun refresh(context: Context) {
            val manager = AppWidgetManager.getInstance(context)
            val ids = manager.getAppWidgetIds(ComponentName(context, AlertWidget::class.java))
            if (ids.isEmpty()) return
            context.sendBroadcast(
                Intent(context, AlertWidget::class.java)
                    .setAction(AppWidgetManager.ACTION_APPWIDGET_UPDATE)
                    .putExtra(AppWidgetManager.EXTRA_APPWIDGET_IDS, ids)
            )
        }
    }
}
