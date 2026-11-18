package io.hookecho.HookEcho

import org.junit.Assert.*
import org.junit.Test

class RadarWidgetTest {
    private val now = 1_800_000_000_000L

    @Test fun aNewlySavedSnapshotDoesNotClaimFreshRadar() {
        val caption = RadarWidget.snapshotAge(now, now)
        assertEquals("Snapshot saved just now\nRadar may be older", caption)
        assertEquals("Snapshot saved 4 min ago\nRadar may be older",
            RadarWidget.snapshotAge(now - 4 * 60_000, now))
    }

    @Test fun oldSnapshotsAndInvalidClocksStayExplicit() {
        assertEquals("Snapshot saved 1 h ago — stale\nRadar may be older",
            RadarWidget.snapshotAge(now - 60 * 60_000, now))
        assertEquals("Old snapshot — open the app\nRadar may be older",
            RadarWidget.snapshotAge(now - 24 * 60 * 60_000, now))
        for (modified in listOf(0L, -1L, now + 60_000)) {
            assertEquals("Snapshot save time unknown — open the app",
                RadarWidget.snapshotAge(modified, now))
        }
    }
}
