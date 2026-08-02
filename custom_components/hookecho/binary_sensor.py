"""Is anything warned here right now — the thing an automation actually triggers on."""

from __future__ import annotations

from homeassistant.components.binary_sensor import (
    BinarySensorDeviceClass,
    BinarySensorEntity,
)
from homeassistant.config_entries import ConfigEntry
from homeassistant.core import HomeAssistant
from homeassistant.helpers.entity_platform import AddEntitiesCallback

from . import HookEchoCoordinator
from .const import (
    DOMAIN,
    KEY_ALERT_ESCALATION,
    KEY_ALERT_EVENT,
    KEY_ALERT_UNTIL,
    KEY_ALERTS,
)
from .sensor import HookEchoEntity


async def async_setup_entry(
    hass: HomeAssistant, entry: ConfigEntry, async_add_entities: AddEntitiesCallback
) -> None:
    coordinator: HookEchoCoordinator = hass.data[DOMAIN][entry.entry_id]
    async_add_entities(HookEchoAlertActive(coordinator, spot) for spot in coordinator.data)


class HookEchoAlertActive(HookEchoEntity, BinarySensorEntity):
    """On while an NWS alert covers, or comes within the watch radius of, this location."""

    _attr_name = "alert"
    _attr_device_class = BinarySensorDeviceClass.SAFETY

    def __init__(self, coordinator: HookEchoCoordinator, spot: str) -> None:
        super().__init__(coordinator, spot)
        self._attr_unique_id = f"hookecho_{spot}_alert_active"

    @property
    def is_on(self) -> bool:
        return bool(self.spot.get(KEY_ALERTS))

    @property
    def extra_state_attributes(self) -> dict:
        alerts = self.spot.get(KEY_ALERTS) or []
        worst = alerts[0] if alerts else {}
        return {
            "event": worst.get(KEY_ALERT_EVENT),
            "until": worst.get(KEY_ALERT_UNTIL),
            # 0 plain, 1 considerable, 2 destructive/observed, 3 tornado emergency or PDS.
            "escalation": worst.get(KEY_ALERT_ESCALATION),
            "count": len(alerts),
        }
