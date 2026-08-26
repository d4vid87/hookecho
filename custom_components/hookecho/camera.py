"""The radar itself, as a still camera — `/snapshot.png` on a dashboard card."""

from __future__ import annotations

import aiohttp
from homeassistant.components.camera import Camera
from homeassistant.config_entries import ConfigEntry
from homeassistant.core import HomeAssistant
from homeassistant.helpers.aiohttp_client import async_get_clientsession
from homeassistant.helpers.device_registry import DeviceInfo
from homeassistant.helpers.entity_platform import AddEntitiesCallback

from . import HookEchoCoordinator
from .const import DOMAIN, snapshot_url


async def async_setup_entry(
    hass: HomeAssistant, entry: ConfigEntry, async_add_entities: AddEntitiesCallback
) -> None:
    coordinator: HookEchoCoordinator = hass.data[DOMAIN][entry.entry_id]
    async_add_entities([HookEchoRadar(coordinator)])


class HookEchoRadar(Camera):
    """One radar image for the server, not one per location — the server picks the site."""

    _attr_has_entity_name = True
    _attr_name = "radar"

    def __init__(self, coordinator: HookEchoCoordinator) -> None:
        super().__init__()
        self._coordinator = coordinator
        host, port = coordinator.host, coordinator.port
        self._attr_unique_id = f"hookecho_{host}_{port}_radar"
        self._attr_device_info = DeviceInfo(
            identifiers={(DOMAIN, f"{host}:{port}")},
            name="HookEcho",
            manufacturer="HookEcho",
            configuration_url=f"http://{host}:{port}/",
        )

    async def async_camera_image(
        self, width: int | None = None, height: int | None = None
    ) -> bytes | None:
        session = async_get_clientsession(self.hass)
        try:
            # A cold render takes seconds; the server then serves it from cache for five minutes.
            async with session.get(
                snapshot_url(self._coordinator.host, self._coordinator.port),
                timeout=aiohttp.ClientTimeout(total=120),
            ) as resp:
                resp.raise_for_status()
                return await resp.read()
        except Exception:  # noqa: BLE001 — a missed frame is not worth an exception trace
            return None
