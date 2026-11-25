package io.hookecho.HookEcho

import org.junit.Assert.*
import org.junit.Test
import java.nio.file.Files
import java.util.Locale
import org.json.JSONArray
import org.json.JSONObject

class NwsTest {
    @Test fun watchedRetainsSamplesBeyondOneRequestBudget() {
        val dir = Files.createTempDirectory("hookecho-coverage-test").toFile()
        try {
            val settings = dir.resolve("config/settings.json")
            settings.parentFile!!.mkdirs()
            fun writePlaces(count: Int, radius: Double) {
                val markers = JSONArray()
                repeat(count) { i -> markers.put(JSONObject().put("name", "Place ${i + 1}")
                    .put("lat", 32.0 + i / 100.0).put("lon", -97.0).put("alert_radius_mi", radius)) }
                settings.writeText(JSONObject().put("markers", markers).toString())
            }
            writePlaces(7, 20.0)
            val seven = Nws.watched(dir)
            assertEquals(7, seven.size)
            assertTrue(seven.all { it.samples.size == 7 })
            for (watch in seven) assertArrayEquals(doubleArrayOf(watch.lat, watch.lon), watch.samples.first(), 0.0)

            writePlaces(42, 0.0)
            val many = Nws.watched(dir)
            assertEquals(42, many.size)
            assertTrue(many.all { it.samples.size == 1 })
            assertEquals("Place 42", many.last().name)
        } finally { dir.deleteRecursively() }
    }

    @Test fun tricklingNetworkBodiesCannotExtendThePassDeadline() {
        serve({ body ->
            body.write("HTTP/1.1 200 OK\r\nContent-Length: 50\r\nConnection: close\r\n\r\n".toByteArray())
            repeat(50) { body.write(' '.code); body.flush(); Thread.sleep(20) }
        }) { url ->
            val start = System.nanoTime()
            val result = runCatching { Nws.fetchFeatures(url, start + 400_000_000L) { true } }
            assertTrue(result.isFailure)
            assertTrue(result.exceptionOrNull()?.message.orEmpty().lowercase().contains("timed out"))
            assertTrue("A trickling response extended the deadline", System.nanoTime() - start < 2_000_000_000L)
        }
    }

    @Test fun cancelledRequestsAndMalformedSuccessesCannotBecomeAllQuiet() {
        val cancelled = runCatching { Nws.fetchFeatures(java.net.URL("http://127.0.0.1:1/"),
            System.nanoTime() + 1_000_000_000L) { false } }
        assertEquals("Alert check cancelled", cancelled.exceptionOrNull()?.message)
        serve({ body ->
            body.write("HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}".toByteArray())
        }) { url ->
            val malformed = runCatching { Nws.fetchFeatures(url, System.nanoTime() + 2_000_000_000L) { true } }
            assertEquals("NWS response has no alert list", malformed.exceptionOrNull()?.message)
        }
    }

    private fun serve(response: (java.io.OutputStream) -> Unit, check: (java.net.URL) -> Unit) {
        java.net.ServerSocket(0, 1, java.net.InetAddress.getByName("127.0.0.1")).use { server ->
            server.soTimeout = 2_000
            val thread = Thread {
                runCatching {
                    server.accept().use { client ->
                        client.soTimeout = 2_000
                        val input = client.getInputStream().bufferedReader()
                        while (!input.readLine().isNullOrEmpty()) { /* Consume request headers. */ }
                        response(client.getOutputStream())
                    }
                }
            }.apply { isDaemon = true; start() }
            try { check(java.net.URL("http://127.0.0.1:${server.localPort}/")) }
            finally { server.close(); thread.join(2_000) }
        }
    }

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

    @Test fun destructiveAndEmergencyWarningsBypassQuietHours() {
        fun tier(event: String, json: String = "{}") = Nws.tierOf(JSONObject(json), event)

        assertEquals(Nws.TIER_EMERGENCY, tier("Severe Thunderstorm Warning",
            """{"parameters":{"thunderstormDamageThreat":["DESTRUCTIVE"]}}"""))
        assertEquals(Nws.TIER_EMERGENCY, tier("Flash Flood Warning",
            """{"description":"...FLASH FLOOD\nEMERGENCY FOR THE CITY..."}"""))
        assertEquals(Nws.TIER_EMERGENCY, tier("Flash Flood Warning",
            """{"parameters":{"flashFloodDamageThreat":["CATASTROPHIC"]}}"""))
        assertEquals(Nws.TIER_EMERGENCY, tier("Tornado Warning",
            """{"parameters":{"tornadoDamageThreat":["CONSIDERABLE"]}}"""))
        assertEquals(Nws.TIER_EMERGENCY, tier("Tornado Warning",
            """{"description":"This is a PARTICULARLY DANGEROUS SITUATION."}"""))
        assertEquals(Nws.TIER_WARNING, tier("Severe Thunderstorm Warning",
            """{"parameters":{"thunderstormDamageThreat":["CONSIDERABLE"]}}"""))
        assertEquals(Nws.TIER_WARNING, tier("Flash Flood Warning"))
        assertEquals(Nws.TIER_WATCH, tier("Tornado Watch",
            """{"description":"Consult local news if a tornado emergency is issued."}"""))
    }

    @Test fun warningRadiusRespectsHolesAndIndependentPolygonParts() {
        val rings = """[[[-101,28],[-93,28],[-93,36],[-101,36],[-101,28]],[[-99,30],[-95,30],[-95,34],[-99,34],[-99,30]]]"""
        val polygon = JSONObject("""{"type":"Polygon","coordinates":$rings}""")
        assertFalse(Nws.geometryTouches(polygon, 32.0, -97.0, 30.0))
        assertTrue(Nws.geometryTouches(polygon, 32.0, -98.9, 30.0))
        assertTrue(Nws.geometryTouches(polygon, 32.0, -100.0, 0.0))
        assertFalse(Nws.geometryTouches(polygon, 32.0, -103.0, 30.0))

        val island = """[[[-97.5,31.5],[-96.5,31.5],[-96.5,32.5],[-97.5,32.5],[-97.5,31.5]]]"""
        val multiPolygon = JSONObject("""{"type":"MultiPolygon","coordinates":[$rings,$island]}""")
        assertTrue(Nws.geometryTouches(multiPolygon, 32.0, -97.0, 30.0))
    }

    @Test fun warningRadiusFollowsTheShortEdgeAcrossTheDateLine() {
        val polygon = JSONObject(
            """{"type":"Polygon","coordinates":[[[179,50],[-179,50],[-179,52],[179,52],[179,50]]]}""",
        )
        assertTrue(Nws.geometryTouches(polygon, 51.0, 179.5, 0.0))
        assertTrue(Nws.geometryTouches(polygon, 51.0, -179.5, 0.0))
        assertTrue(Nws.geometryTouches(polygon, 51.0, -178.9, 30.0))
        assertFalse(Nws.geometryTouches(polygon, 51.0, 0.0, 30.0))
    }

    @Test fun watchZoneQueryAndDestinationStayInsideConcaveAndDatelineRings() {
        val dir = Files.createTempDirectory("hookecho-watch-zone-test").toFile()
        try {
            val settings = dir.resolve("config/settings.json")
            settings.parentFile!!.mkdirs()
            val dateline = """[[179,50],[-179,50],[-179,52],[179,52],[179,50]]"""
            val concave = """[[-101,30],[-97,30],[-97,34],[-98,34],[-98,31],[-100,31],[-100,34],[-101,34],[-101,30]]"""
            val small = """[[-99.23456,32.123461],[-99.23455,32.123461],[-99.23455,32.123469],[-99.23456,32.123469],[-99.23456,32.123461]]"""
            for (ring in listOf(dateline, concave, small)) {
                settings.writeText("""{"alert_polygons":[{"name":"Zone","ring":$ring}]}""")
                val watch = Nws.watched(dir).single()
                val polygon = JSONObject("""{"type":"Polygon","coordinates":[$ring]}""")
                val (sampleLat, sampleLon) = watch.samples.first()
                val (queryLat, queryLon) = Nws.pointUrl(sampleLat, sampleLon).substringAfter("point=")
                    .split(',').map(String::toDouble)
                val (_, gotoLon, gotoLat) = Nws.gotoPoint(watch.lon, watch.lat).split(',')
                assertTrue("Query $queryLon, $queryLat is outside $ring",
                    Nws.geometryTouches(polygon, queryLat, queryLon, 0.0))
                assertTrue("Tap destination $gotoLon, $gotoLat is outside $ring",
                    Nws.geometryTouches(polygon, gotoLat.toDouble(), gotoLon.toDouble(), 0.0))
            }
        } finally { dir.deleteRecursively() }
    }
}
