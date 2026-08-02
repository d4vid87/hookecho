"""Config flow: ask for host and port, then prove something answers there."""

from __future__ import annotations

from typing import Any

import aiohttp
import voluptuous as vol
from homeassistant.config_entries import ConfigFlow, ConfigFlowResult
from homeassistant.helpers.aiohttp_client import async_get_clientsession

from .const import CONF_HOST, CONF_PORT, DEFAULT_PORT, DOMAIN, status_url

SCHEMA = vol.Schema(
    {
        vol.Required(CONF_HOST, default="127.0.0.1"): str,
        vol.Required(CONF_PORT, default=DEFAULT_PORT): int,
    }
)


class HookEchoConfigFlow(ConfigFlow, domain=DOMAIN):
    """Handle a config flow for Hook Echo-WX."""

    VERSION = 1

    async def async_step_user(
        self, user_input: dict[str, Any] | None = None
    ) -> ConfigFlowResult:
        errors: dict[str, str] = {}
        if user_input is not None:
            host, port = user_input[CONF_HOST], user_input[CONF_PORT]
            await self.async_set_unique_id(f"{host}:{port}")
            self._abort_if_unique_id_configured()
            error = await self._probe(host, port)
            if error is None:
                return self.async_create_entry(
                    title=f"Hook Echo-WX ({host})", data=user_input
                )
            errors["base"] = error

        return self.async_show_form(
            step_id="user", data_schema=SCHEMA, errors=errors
        )

    async def _probe(self, host: str, port: int) -> str | None:
        """`None` when the endpoint answers with a list of locations."""
        session = async_get_clientsession(self.hass)
        try:
            async with session.get(
                status_url(host, port), timeout=aiohttp.ClientTimeout(total=30)
            ) as resp:
                resp.raise_for_status()
                spots = await resp.json(content_type=None)
        except Exception:  # noqa: BLE001 — everything here means "can't talk to it"
            return "cannot_connect"
        if not isinstance(spots, list):
            return "invalid_response"
        # An empty list is a working server with no saved locations — worth saying so plainly,
        # since every entity this integration creates comes from that list.
        return None if spots else "no_locations"
