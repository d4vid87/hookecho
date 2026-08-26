"""HookEcho integration: poll a local `hookecho --serve` for conditions and alerts."""

from __future__ import annotations

import logging

import aiohttp
from homeassistant.config_entries import ConfigEntry
from homeassistant.const import Platform
from homeassistant.core import HomeAssistant
from homeassistant.helpers.aiohttp_client import async_get_clientsession
from homeassistant.helpers.update_coordinator import DataUpdateCoordinator, UpdateFailed

from .const import CONF_HOST, CONF_PORT, DOMAIN, KEY_NAME, UPDATE_INTERVAL, status_url

_LOGGER = logging.getLogger(__name__)

PLATFORMS = [Platform.SENSOR, Platform.BINARY_SENSOR, Platform.CAMERA]


class HookEchoCoordinator(DataUpdateCoordinator):
    """Fetches `/status.json` and hands out one dict per location, keyed by name."""

    def __init__(self, hass: HomeAssistant, host: str, port: int) -> None:
        super().__init__(
            hass, _LOGGER, name=DOMAIN, update_interval=UPDATE_INTERVAL
        )
        self.host = host
        self.port = port
        self._session = async_get_clientsession(hass)

    async def _async_update_data(self) -> dict[str, dict]:
        try:
            async with self._session.get(
                status_url(self.host, self.port),
                timeout=aiohttp.ClientTimeout(total=30),
            ) as resp:
                resp.raise_for_status()
                spots = await resp.json(content_type=None)
        except Exception as err:  # noqa: BLE001 — any failure is one failed poll
            raise UpdateFailed(f"hookecho at {self.host}:{self.port}: {err}") from err

        # The server caches for a minute, so polling on the same clock is free.
        return {spot[KEY_NAME]: spot for spot in spots if KEY_NAME in spot}


async def async_setup_entry(hass: HomeAssistant, entry: ConfigEntry) -> bool:
    coordinator = HookEchoCoordinator(
        hass, entry.data[CONF_HOST], entry.data[CONF_PORT]
    )
    await coordinator.async_config_entry_first_refresh()
    hass.data.setdefault(DOMAIN, {})[entry.entry_id] = coordinator
    await hass.config_entries.async_forward_entry_setups(entry, PLATFORMS)
    return True


async def async_unload_entry(hass: HomeAssistant, entry: ConfigEntry) -> bool:
    unloaded = await hass.config_entries.async_unload_platforms(entry, PLATFORMS)
    if unloaded:
        hass.data[DOMAIN].pop(entry.entry_id)
    return unloaded
