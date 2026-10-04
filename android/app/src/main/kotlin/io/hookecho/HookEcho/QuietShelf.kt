package io.hookecho.HookEcho

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp

private val navy = Color(0xFF10273C)
private val rowColor = Color(0xFF1A3954)
private val cobalt = Color(0xFF2C70C4)
private val sky = Color(0xFF8FCDF9)
private val muted = Color(0xFFAAC3D5)
private val menus = listOf("Radar", "Layers", "Alerts", "More")

data class ShelfAction(val title: String, val detail: String, val glyph: String, val command: String)

private fun actions(section: String, site: String, product: String, alerts: Int): List<ShelfAction> = when (section) {
    "Radar" -> listOf(
        ShelfAction("$site · $product", "Current radar and products", "◉", "radar"),
        ShelfAction("Choose radar", "Nearby sites and radar lens", "⌖", "radar"),
        ShelfAction("Radar products", "Reflectivity, velocity and more", "▦", "radar"),
        ShelfAction("Playback", "Time range and animation", "▶", "radar")
    )
    "Layers" -> listOf(
        ShelfAction("Weather layers", "MRMS, outlooks and storm tracks", "▱", "layers"),
        ShelfAction("Map reference", "Alerts, radar sites and basemap", "⌖", "layers"),
        ShelfAction("All layers", "Search the complete library", "▦", "layers")
    )
    "Alerts" -> listOf(
        ShelfAction("Alerts in view", "$alerts active near this map", "⚠", "alerts"),
        ShelfAction("Alert rules", "Choose warning types and sounds", "⚙", "alert_rules")
    )
    else -> listOf(
        ShelfAction("Custom locations", "Home, Work, Car and saved places", "⌂", "markers"),
        ShelfAction("Point forecast", "Tap a place on the map", "⊕", "forecast"),
        ShelfAction("Storm attributes", "Cell strength and motion", "↗", "storms"),
        ShelfAction("Sensor dashboard", "Temperature and trend", "▦", "sensors"),
        ShelfAction("Settings", "Radar, display and notifications", "⚙", "settings"),
        ShelfAction("Help", "Controls and guide", "?", "help"),
        ShelfAction("Analyst workstation", "Advanced tools and comparisons", "◫", "analyst")
    )
}

@Composable
fun QuietShelf(
    section: String,
    site: String,
    product: String,
    alerts: Int,
    onSection: (String) -> Unit,
    onClose: () -> Unit,
    onAction: (String) -> Unit
) {
    BoxWithConstraints(
        Modifier.fillMaxSize()
            .background(Color(0x99000813))
            .clickable(onClick = onClose),
        contentAlignment = Alignment.BottomCenter
    ) {
        Surface(
            modifier = Modifier.fillMaxWidth().height(minOf(530.dp, maxHeight * 0.82f)).clickable {},
            shape = RoundedCornerShape(topStart = 22.dp, topEnd = 22.dp),
            color = navy,
            shadowElevation = 18.dp
        ) {
            Column(Modifier.navigationBarsPadding().padding(horizontal = 14.dp)) {
                Box(Modifier.fillMaxWidth().padding(top = 9.dp), contentAlignment = Alignment.Center) {
                    Box(Modifier.size(width = 44.dp, height = 4.dp).background(Color(0xFF748B9B), RoundedCornerShape(4.dp)))
                }
                Row(
                    Modifier.fillMaxWidth().padding(top = 13.dp, bottom = 11.dp),
                    verticalAlignment = Alignment.CenterVertically
                ) {
                    Column(Modifier.weight(1f)) {
                        Text("HOOKECHO / MOBILE", color = sky, fontSize = 10.sp, fontWeight = FontWeight.Bold, letterSpacing = 1.sp)
                        Text(section, color = Color.White, fontSize = 22.sp, fontWeight = FontWeight.SemiBold)
                    }
                    Surface(shape = RoundedCornerShape(9.dp), color = rowColor, modifier = Modifier.clickable(onClick = onClose)) {
                        Text("×", color = Color.White, fontSize = 24.sp, modifier = Modifier.padding(horizontal = 14.dp, vertical = 4.dp))
                    }
                }
                Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.spacedBy(5.dp)) {
                    menus.forEach { name ->
                        Surface(
                            modifier = Modifier.weight(1f).height(40.dp).clickable { onSection(name) },
                            shape = RoundedCornerShape(8.dp),
                            color = if (name == section) cobalt else rowColor
                        ) {
                            Box(contentAlignment = Alignment.Center) {
                                Text(name, color = Color.White, fontSize = 12.sp, fontWeight = if (name == section) FontWeight.Bold else FontWeight.Normal)
                            }
                        }
                    }
                }
                Text("${section.uppercase()} / QUICK ACTIONS", color = sky, fontSize = 10.sp, fontWeight = FontWeight.Bold,
                    letterSpacing = 1.sp, modifier = Modifier.padding(top = 18.dp, bottom = 7.dp))
                LazyColumn(verticalArrangement = Arrangement.spacedBy(7.dp)) {
                    items(actions(section, site, product, alerts)) { item ->
                        Surface(
                            modifier = Modifier.fillMaxWidth().height(60.dp).clickable { onAction(item.command) },
                            shape = RoundedCornerShape(11.dp),
                            color = if (item.command == "markers") cobalt else rowColor
                        ) {
                            Row(Modifier.padding(horizontal = 11.dp), verticalAlignment = Alignment.CenterVertically) {
                                Text(item.glyph, color = sky, fontSize = 21.sp, modifier = Modifier.size(33.dp))
                                Column(Modifier.weight(1f)) {
                                    Text(item.title, color = Color.White, fontSize = 14.sp, fontWeight = FontWeight.SemiBold)
                                    Text(item.detail, color = muted, fontSize = 11.sp)
                                }
                                Text("›", color = muted, fontSize = 20.sp)
                            }
                        }
                    }
                    item { Spacer(Modifier.height(12.dp)) }
                }
            }
        }
    }
}
