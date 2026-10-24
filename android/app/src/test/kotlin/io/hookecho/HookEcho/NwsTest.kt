package io.hookecho.HookEcho

import org.junit.Assert.*
import org.junit.Test
import java.nio.file.Files
import java.util.Locale

class NwsTest {
    @Test fun coordinatesAndNotificationNavigationIgnorePhoneLocale() {
        val original = Locale.getDefault()
        try {
            for (locale in listOf(Locale.US, Locale.GERMANY, Locale.FRANCE)) {
                Locale.setDefault(locale)
                assertEquals("https://api.weather.gov/alerts/active?point=33.0665,-96.0899", Nws.pointUrl(33.0665, -96.0899))
                assertEquals(",-96.0899,33.0665,9", Nws.gotoPoint(-96.0899, 33.0665))
            }
        } finally { Locale.setDefault(original) }
    }

    @Test fun partialFailuresNeverClaimAllQuietAndKeepKnownWarnings() {
        val partial = Nws.checkSummary(true, emptyList(), 1, 1, 0)
        assertTrue(partial.contains("Some locations couldn't be checked"))
        assertFalse(partial.contains("All quiet"))
        val warning = "Tornado Warning — Home"
        val known = Nws.checkSummary(true, listOf(warning, warning), 1, 1, 123456789)
        assertTrue(known.startsWith(warning))
        assertEquals(1, known.lines().count { it == warning })
        assertTrue(known.contains("Last complete check:"))
        assertEquals("Couldn't check alerts", Nws.checkSummary(true, emptyList(), 0, 2, 0))
        assertEquals("Couldn't check alerts", Nws.checkSummary(true, emptyList(), 0, 0, 0))
        assertEquals("All quiet", Nws.checkSummary(true, emptyList(), 2, 0, 0))
        assertEquals("No saved locations yet", Nws.checkSummary(false, emptyList(), 0, 0, 0))
    }

    @Test fun backgroundQuietHoursWrapMidnightAndSurviveSettingsReload() {
        val dir = Files.createTempDirectory("hookecho-alert-test").toFile()
        try {
            val settings = dir.resolve("config/settings.json")
            settings.parentFile!!.mkdirs()
            settings.writeText("""{"quiet_hours":true,"quiet_start_hour":22,"quiet_end_hour":7,"markers":[{"name":"Home","lat":33.0665,"lon":-96.0899,"home":true}]}""")
            assertTrue(Nws.inQuietHours(dir, 23))
            assertTrue(Nws.inQuietHours(dir, 6))
            assertFalse(Nws.inQuietHours(dir, 7))
            assertFalse(Nws.inQuietHours(dir, 21))
            assertEquals("Home", Nws.watched(dir).single().name)
            assertTrue(Nws.watched(dir).single().home)
            settings.writeText("""{"quiet_hours":true,"quiet_start_hour":7,"quiet_end_hour":7}""")
            assertFalse(Nws.inQuietHours(dir, 7))
        } finally { dir.deleteRecursively() }
    }
}
