package io.hookecho.HookEcho

import android.app.PendingIntent
import android.appwidget.AppWidgetManager
import android.appwidget.AppWidgetProvider
import android.content.ComponentName
import android.content.Context
import android.content.Intent
import android.graphics.BitmapFactory
import android.widget.RemoteViews
import java.io.File

/**
 * Home-screen widget showing the last radar view the app drew.
 *
 * The widget does no network and no rendering: it cannot host the Rust renderer, and a widget that
 * fetched and decoded a volume on the home screen would be a second radar client running on
 * someone's battery. Instead the app writes a small PNG whenever a scan lands
 * ([SNAPSHOT] in its files dir) and this reads whatever is there, captioned with the age — a
 * picture with no timestamp is worse than no picture, because it looks current.
 */
class RadarWidget : AppWidgetProvider() {
    override fun onUpdate(context: Context, manager: AppWidgetManager, ids: IntArray) {
        render(context, manager, ids)
    }

    private fun render(context: Context, manager: AppWidgetManager, ids: IntArray) {
        val png = File(context.filesDir, SNAPSHOT)
        val views = RemoteViews(context.packageName, R.layout.widget_radar)
        if (png.exists()) {
            runCatching { BitmapFactory.decodeFile(png.absolutePath) }.getOrNull()?.let {
                views.setImageViewBitmap(R.id.widget_radar_image, it)
            }
            views.setTextViewText(R.id.widget_radar_text, caption(context, png.lastModified()))
        } else {
            views.setTextViewText(R.id.widget_radar_text, "Open HookEcho to fill this in")
        }
        val tap = PendingIntent.getActivity(
            context,
            0,
            Intent(context, MainActivity::class.java)
                .setFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_SINGLE_TOP),
            PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE,
        )
        views.setOnClickPendingIntent(R.id.widget_radar_root, tap)
        for (id in ids) manager.updateAppWidget(id, views)
    }

    /**
     * The line under the picture: the nearest storm and how far off it is, then the age.
     *
     * The app writes the storm half ([CAPTION]) whenever it writes the picture, because the
     * widget has no cell data of its own and is not going to fetch any on someone's home screen.
     * No file means nothing is being tracked near them, and the age stands alone.
     */
    private fun caption(context: Context, modified: Long): String {
        val storm = runCatching {
            File(context.filesDir, CAPTION).takeIf { it.exists() }?.readText()?.trim()
        }.getOrNull()
        val age = age(modified)
        return if (storm.isNullOrEmpty()) age else "$storm\n$age"
    }

    /** "4 min ago", and blunt about it once the picture is old enough to mislead. */
    private fun age(modified: Long): String {
        val mins = (System.currentTimeMillis() - modified) / 60_000
        return when {
            mins < 1 -> "just now"
            mins < 60 -> "$mins min ago"
            mins < 60 * 24 -> "${mins / 60} h ago — stale"
            else -> "old — open the app"
        }
    }

    companion object {
        /** Where the Rust side writes the picture, relative to `filesDir`. */
        const val SNAPSHOT = "widget-radar.png"

        /** Where the Rust side writes the storm line that goes under it. */
        const val CAPTION = "widget-radar.txt"

        /** Re-render every placed widget now. Called from Rust after it writes a new snapshot. */
        @JvmStatic
        fun refresh(context: Context) {
            val manager = AppWidgetManager.getInstance(context)
            val ids = manager.getAppWidgetIds(ComponentName(context, RadarWidget::class.java))
            if (ids.isEmpty()) return
            context.sendBroadcast(
                Intent(context, RadarWidget::class.java)
                    .setAction(AppWidgetManager.ACTION_APPWIDGET_UPDATE)
                    .putExtra(AppWidgetManager.EXTRA_APPWIDGET_IDS, ids)
            )
        }
    }
}
