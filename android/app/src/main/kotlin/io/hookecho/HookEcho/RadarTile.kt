package io.hookecho.HookEcho

import android.content.Intent
import android.service.quicksettings.TileService

/**
 * Quick-settings tile: open the radar from the pull-down shade.
 *
 * Deliberately nothing more. A tile that showed live warning state would need its own polling, and
 * that already exists as the alert service and the widget; this is the two-swipe path to the map
 * when the sky goes green.
 */
class RadarTile : TileService() {
    override fun onClick() {
        super.onClick()
        val intent = Intent(this, MainActivity::class.java)
            .setFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_SINGLE_TOP)
        // startActivityAndCollapse wants a PendingIntent from Android 14 on, and refuses an
        // Intent there; below that it only takes the Intent.
        if (android.os.Build.VERSION.SDK_INT >= android.os.Build.VERSION_CODES.UPSIDE_DOWN_CAKE) {
            startActivityAndCollapse(
                android.app.PendingIntent.getActivity(
                    this,
                    0,
                    intent,
                    android.app.PendingIntent.FLAG_UPDATE_CURRENT or
                        android.app.PendingIntent.FLAG_IMMUTABLE,
                )
            )
        } else {
            @Suppress("DEPRECATION")
            startActivityAndCollapse(intent)
        }
    }
}
