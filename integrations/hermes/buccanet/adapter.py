"""Buccanet platform adapter for Hermes Agent.

Each Buccanet room is a group chat. Incoming messages stream from the local
`bucca serve-bridge` (Server Sent Events); replies are posted back to it. The
bridge owns the profile, so MLS keys never enter the Hermes process. Standard
library only, like the IRC adapter: nothing to install.
"""

import asyncio
import datetime
import json
import logging
import threading
import urllib.request
import uuid
from typing import Any, Optional

from gateway.config import Platform
from gateway.platforms._shared import get_scoped_secret as _get_scoped_secret
from gateway.platforms.base import BasePlatformAdapter, SendResult, resolve_channel_prompt
from gateway.platforms.event import MessageEvent, MessageType

logger = logging.getLogger(__name__)

RECONNECT_SECONDS = 3.0
MAX_MESSAGE_LENGTH = 6000
# The guidance every Buccanet harness gets. A room can set its own through
# gateway.platforms.buccanet.extra.channel_prompts.<room id>.
GROUP_PROMPT = (
    "You are in a Buccanet group chat with people and other AI agents. Answer when you are addressed "
    "or when you have something useful to add. Do not acknowledge or repeat other agents' messages. "
    "When you have nothing to add, reply with exactly [SILENT]."
)


def _bridge_settings(extra: dict) -> tuple[str, str]:
    url = _get_scoped_secret("BUCCANET_BRIDGE_URL", "") or extra.get("bridge_url", "")
    token = _get_scoped_secret("BUCCANET_BRIDGE_TOKEN", "") or extra.get("bridge_token", "")
    return url.rstrip("/"), token


class BuccanetAdapter(BasePlatformAdapter):
    """Relays Buccanet rooms to the Hermes agent through the local bridge."""

    serial_receive_loop = True

    def __init__(self, config, **kwargs):
        super().__init__(config=config, platform=Platform("buccanet"))
        self.extra = getattr(config, "extra", {}) or {}
        self.bridge_url, self.bridge_token = _bridge_settings(self.extra)
        self._loop: Optional[asyncio.AbstractEventLoop] = None
        self._stop = threading.Event()
        self._reader: Optional[threading.Thread] = None
        self._titles: dict[str, str] = {}

    @property
    def name(self) -> str:
        return "Buccanet"

    def _request(self, method: str, path: str, body: Optional[dict] = None, timeout: Optional[float] = 15):
        data = json.dumps(body).encode() if body is not None else None
        request = urllib.request.Request(f"{self.bridge_url}{path}", data=data, method=method)
        request.add_header("Authorization", f"Bearer {self.bridge_token}")
        if data is not None:
            request.add_header("Content-Type", "application/json")
        return urllib.request.urlopen(request, timeout=timeout)

    def _get_json(self, path: str) -> dict:
        with self._request("GET", path) as response:
            return json.loads(response.read())

    async def connect(self, *, is_reconnect: bool = False) -> bool:
        if not self.bridge_url or not self.bridge_token:
            missing = "BUCCANET_BRIDGE_URL and BUCCANET_BRIDGE_TOKEN must be set"
            self._set_fatal_error("config_missing", missing, retryable=False)
            return False
        try:
            rooms = (await asyncio.to_thread(self._get_json, "/rooms")).get("rooms", [])
        except Exception as error:
            logger.error("Buccanet: bridge unreachable at %s: %s", self.bridge_url, error)
            self._set_fatal_error("connect_failed", str(error), retryable=True)
            return False
        self._titles = {room["roomId"]: room.get("title") or room["roomId"] for room in rooms}
        self._loop = asyncio.get_running_loop()
        self._stop.clear()
        self._reader = threading.Thread(target=self._read_stream, name="buccanet-bridge", daemon=True)
        self._reader.start()
        self._mark_connected()
        self._wire_plugin_handlers(None)
        logger.info("Buccanet: connected to bridge %s (%d rooms)", self.bridge_url, len(self._titles))
        return True

    async def disconnect(self) -> None:
        self._mark_disconnected()
        self._stop.set()

    async def send(
        self, chat_id: str, content: str, reply_to: Optional[str] = None, metadata: Optional[dict[str, Any]] = None
    ):
        body = {"roomId": chat_id, "text": content[:MAX_MESSAGE_LENGTH]}

        def post() -> int:
            with self._request("POST", "/send", body) as response:
                return json.loads(response.read()).get("seq")

        try:
            seq = await asyncio.to_thread(post)
        except Exception as error:
            return SendResult(success=False, error=str(error))
        return SendResult(success=True, message_id=f"{chat_id}:{seq}")

    async def send_typing(self, chat_id: str, metadata=None) -> None:
        """Buccanet has no typing indicator yet."""

    async def get_chat_info(self, chat_id: str) -> dict[str, Any]:
        return {"name": self._titles.get(chat_id, chat_id), "type": "group", "chat_id": chat_id}

    def _read_stream(self) -> None:
        """Reader thread: follows the bridge's event stream and hands each item to the event loop."""
        while not self._stop.is_set():
            try:
                with self._request("GET", "/events", timeout=None) as response:
                    data: list[str] = []
                    for raw in response:
                        if self._stop.is_set():
                            return
                        line = raw.decode("utf-8").rstrip("\r\n")
                        if line.startswith("data:"):
                            data.append(line[5:].lstrip())
                        elif line == "" and data:
                            item = json.loads("\n".join(data))
                            data = []
                            asyncio.run_coroutine_threadsafe(self._on_item(item), self._loop)
            except Exception as error:
                logger.warning("Buccanet: bridge stream dropped: %s", error)
            self._stop.wait(RECONNECT_SECONDS)

    async def _on_item(self, item: dict) -> None:
        room_id = item.get("roomId", "")
        if item.get("roomTitle"):
            self._titles[room_id] = item["roomTitle"]
        if item.get("type") != "message" or not self._message_handler:
            return
        author = item.get("from", {})
        source = self.build_source(
            chat_id=room_id,
            chat_name=self._titles.get(room_id, room_id),
            chat_type="group",
            user_id=author.get("deviceId"),
            user_name=author.get("name"),
            is_bot=author.get("kind") == "agent",
            message_id=f"{room_id}:{item.get('seq')}",
        )
        event = MessageEvent(
            text=item.get("text", ""),
            message_type=MessageType.TEXT,
            source=source,
            message_id=uuid.uuid4().hex,
            timestamp=datetime.datetime.now(),
            channel_prompt=resolve_channel_prompt(self.extra, room_id) or GROUP_PROMPT,
        )
        await self.handle_message(event)


def check_requirements() -> bool:
    return bool(_get_scoped_secret("BUCCANET_BRIDGE_URL", "") and _get_scoped_secret("BUCCANET_BRIDGE_TOKEN", ""))


def validate_config(config) -> bool:
    url, token = _bridge_settings(getattr(config, "extra", {}) or {})
    return bool(url and token)


def _env_enablement() -> dict | None:
    url = _get_scoped_secret("BUCCANET_BRIDGE_URL", "").strip()
    token = _get_scoped_secret("BUCCANET_BRIDGE_TOKEN", "").strip()
    return {"bridge_url": url, "bridge_token": token} if url and token else None


def register(ctx):
    """Plugin entry point: called by the Hermes plugin system."""
    ctx.register_platform(
        name="buccanet",
        dashboard_description="Join end to end encrypted Buccanet rooms shared by agents and humans.",
        label="Buccanet",
        adapter_factory=BuccanetAdapter,
        check_fn=check_requirements,
        validate_config=validate_config,
        is_connected=validate_config,
        required_env=["BUCCANET_BRIDGE_URL", "BUCCANET_BRIDGE_TOKEN"],
        install_hint=(
            "Run `bucca serve-bridge --profile <name>` next to the gateway, with the token in "
            "BUCCANET_BRIDGE_TOKEN or in a file passed as --token-file <path>"
        ),
        env_enablement_fn=_env_enablement,
        cron_deliver_env_var="BUCCANET_HOME_CHANNEL",
        allowed_users_env="BUCCANET_ALLOWED_USERS",
        allow_all_env="BUCCANET_ALLOW_ALL_USERS",
        max_message_length=MAX_MESSAGE_LENGTH,
        emoji="◆",
        pii_safe=True,
        platform_hint=(
            "You are chatting in a Buccanet room: an end to end encrypted group chat shared by AI agents "
            "and humans. Address people by name, keep messages short, and stay silent when others already "
            "covered the point. Plain text works best."
        ),
    )
