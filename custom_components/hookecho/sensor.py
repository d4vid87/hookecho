"""One device per saved location, with its current conditions and alert count."""

from __future__ import annotations

from homeassistant.components.sensor import SensorEntity
from homeassistant.config_entries import ConfigEntry
from homeassistant.core import HomeAssistant
from homeassistant.helpers.device_registry import DeviceInfo
from homeassistant.helpers.entity_platform import AddEntitiesCallback
from homeassistant.helpers.update_coordinator import CoordinatorEntity

from . import HookEchoCoordinator
from .const import (
    DOMAIN,
    KEY_ALERT_DISTANCE,
    KEY_ALERT_ESCALATION,
    KEY_ALERT_EVENT,
    KEY_ALERT_UNTIL,
    KEY_ALERTS,
    KEY_STATION,
    MEASUREMENTS,
)


async def async_setup_entry(
    hass: HomeAssistant, entry: ConfigEntry, async_add_entities: AddEntitiesCallback
) -> None:
    coordinator: HookEchoCoordinator = hass.data[DOMAIN][entry.entry_id]
    entities: list[SensorEntity] = []
    for spot in coordinator.data:
        entities += [
            HookEchoMeasurement(coordinator, spot, *m) for m in MEASUREMENTS
        ]
        entities.append(HookEchoAlertCount(coordinator, spot))
    async_add_entities(entities)


def device_info(coordinator: HookEchoCoordinator, spot: str) -> DeviceInfo:
    return DeviceInfo(
        identifiers={(DOMAIN, f"{coordinator.host}:{coordinator.port}:{spot}")},
        name=f"Hook Echo-WX {spot}",
        manufacturer="Hook Echo-WX",
        configuration_url=f"http://{coordinator.host}:{coordinator.port}/",
    )


class HookEchoEntity(CoordinatorEntity[HookEchoCoordinator]):
    """Shared plumbing: which location this entity speaks for."""

    _attr_has_entity_name = True

    def __init__(self, coordinator: HookEchoCoordinator, spot: str) -> None:
        super().__init__(coordinator)
        self._spot = spot
        self._attr_device_info = device_info(coordinator, spot)

    @property
    def spot(self) -> dict:
        return self.coordinator.data.get(self._spot, {})

    @property
    def available(self) -> bool:
        # A location that vanished from the report (marker deleted) is unavailable, not stale.
        return super().available and self._spot in self.coordinator.data


class HookEchoMeasurement(HookEchoEntity, SensorEntity):
    """Temperature, humidity, wind — whatever the nearest station reported."""

    def __init__(
        self,
        coordinator: HookEchoCoordinator,
        spot: str,
        json_key: str,
        key: str,
        label: str,
        unit: str,
        device_class: str,
        state_class: str,
    ) -> None:
        super().__init__(coordinator, spot)
        self._json_key = json_key
        self._attr_name = label
        self._attr_unique_id = f"hookecho_{spot}_{key}"
        self._attr_native_unit_of_measurement = unit
        self._attr_device_class = device_class
        self._attr_state_class = state_class

    @property
    def native_value(self) -> float | None:
        # Absent is absent: a station that didn't report humidity must not read as 0%.
        return self.spot.get(self._json_key)

    @property
    def extra_state_attributes(self) -> dict:
        return {"station": self.spot.get(KEY_STATION)}


class HookEchoAlertCount(HookEchoEntity, SensorEntity):
    """How many alerts are within this location's watch radius, worst one detailed."""

    _attr_name = "alerts"
    _attr_icon = "mdi:weather-lightning"
    _attr_state_class = "measurement"

    def __init__(self, coordinator: HookEchoCoordinator, spot: str) -> None:
        super().__init__(coordinator, spot)
        self._attr_unique_id = f"hookecho_{spot}_alerts"

    @property
    def native_value(self) -> int:
        return len(self.spot.get(KEY_ALERTS) or [])

    @property
    def extra_state_attributes(self) -> dict:
        alerts = self.spot.get(KEY_ALERTS) or []
        # The report is sorted worst-first, so the head is the one an automation should act on.
        worst = alerts[0] if alerts else {}
        return {
            "events": [a.get(KEY_ALERT_EVENT) for a in alerts],
            "event": worst.get(KEY_ALERT_EVENT),
            "until": worst.get(KEY_ALERT_UNTIL),
            "distance_km": worst.get(KEY_ALERT_DISTANCE),
            "escalation": worst.get(KEY_ALERT_ESCALATION),
        }
