from __future__ import annotations

import dataclasses
import datetime as dt
import hashlib
import json
import logging
import logging.handlers
import os
import platform
import shutil
import signal
import socket
import sqlite3
import subprocess
import sys
import threading
import time
from collections import deque
from pathlib import Path
from typing import Any

import requests
import tomllib

try:
    import psutil  # type: ignore
except ImportError:
    psutil = None


class InternalVoiceError(RuntimeError):
    pass


@dataclasses.dataclass
class ServiceConfig:
    sample_interval_secs: int
    gemini_timeout_secs: int
    gemini_model: str
    gemini_api_key: str
    debounce_millis: int
    max_prompt_tokens_hint: int
    cache_ttl_secs: int
    max_log_files: int
    log_dir: str
    max_log_file_size_mb: int


@dataclasses.dataclass
class SecretConfig:
    env_var: str
    keyring_service: str
    keyring_account: str


@dataclasses.dataclass
class LimitConfig:
    requests_per_minute: int
    max_cpu_percent: float
    max_memory_mb: int
    breaker_failure_threshold: int
    breaker_reset_secs: int
    system_telemetry_interval_secs: int


@dataclasses.dataclass
class NarrationConfig:
    min_announcement_interval_secs: int
    voice_enabled: bool
    max_processes: int
    style: str


@dataclasses.dataclass
class PolicyConfig:
    notify_on_battery_below: float
    notify_on_disk_below: float
    notify_on_cpu_above: float
    notify_on_memory_above: float
    system_telemetry: dict[str, Any] | None


@dataclasses.dataclass
class AppConfig:
    service: ServiceConfig
    secrets: SecretConfig
    limits: LimitConfig
    narration: NarrationConfig
    policy: PolicyConfig

    @classmethod
    def load(cls) -> "AppConfig":
        path = config_path()
        try:
            payload = tomllib.loads(path.read_text())
        except FileNotFoundError as err:
            raise InternalVoiceError(f"configuration error: failed reading config {path}: {err}") from err
        except tomllib.TOMLDecodeError as err:
            raise InternalVoiceError(f"configuration error: invalid config: {err}") from err

        try:
            service = payload["service"]
            secrets = payload["secrets"]
            limits = payload["limits"]
            narration = payload["narration"]
            policy = payload["policy"]
        except KeyError as err:
            raise InternalVoiceError(f"configuration error: missing top-level config section: {err.args[0]}") from err

        return cls(
            service=ServiceConfig(
                sample_interval_secs=int(service["sample_interval_secs"]),
                gemini_timeout_secs=int(service["gemini_timeout_secs"]),
                gemini_model=str(service["gemini_model"]),
                gemini_api_key=str(service.get("gemini_api_key", "")),
                debounce_millis=int(service["debounce_millis"]),
                max_prompt_tokens_hint=int(service["max_prompt_tokens_hint"]),
                cache_ttl_secs=int(service["cache_ttl_secs"]),
                max_log_files=int(service["max_log_files"]),
                log_dir=str(service.get("log_dir", "logs")),
                max_log_file_size_mb=int(service["max_log_file_size_mb"]),
            ),
            secrets=SecretConfig(
                env_var=str(secrets["env_var"]),
                keyring_service=str(secrets["keyring_service"]),
                keyring_account=str(secrets["keyring_account"]),
            ),
            limits=LimitConfig(
                requests_per_minute=int(limits["requests_per_minute"]),
                max_cpu_percent=float(limits["max_cpu_percent"]),
                max_memory_mb=int(limits["max_memory_mb"]),
                breaker_failure_threshold=int(limits["breaker_failure_threshold"]),
                breaker_reset_secs=int(limits["breaker_reset_secs"]),
                system_telemetry_interval_secs=int(limits.get("system_telemetry_interval_secs", 60)),
            ),
            narration=NarrationConfig(
                min_announcement_interval_secs=int(narration["min_announcement_interval_secs"]),
                voice_enabled=bool(narration["voice_enabled"]),
                max_processes=int(narration["max_processes"]),
                style=str(narration["style"]),
            ),
            policy=PolicyConfig(
                notify_on_battery_below=float(policy["notify_on_battery_below"]),
                notify_on_disk_below=float(policy["notify_on_disk_below"]),
                notify_on_cpu_above=float(policy["notify_on_cpu_above"]),
                notify_on_memory_above=float(policy["notify_on_memory_above"]),
                system_telemetry=policy.get("system_telemetry"),
            ),
        )

    def sample_interval(self) -> float:
        return float(self.service.sample_interval_secs)

    def announcement_interval(self) -> float:
        return float(self.narration.min_announcement_interval_secs)

    def gemini_timeout(self) -> float:
        return float(self.service.gemini_timeout_secs)

    def breaker_reset(self) -> float:
        return float(self.limits.breaker_reset_secs)

    def debounce_interval(self) -> float:
        return self.service.debounce_millis / 1000.0

    def system_telemetry_interval(self) -> float:
        return float(self.limits.system_telemetry_interval_secs)

    def log_dir_path(self) -> Path:
        return Path(self.service.log_dir)


def load_dotenv() -> None:
    env_path = Path(".env")
    if not env_path.exists():
        return

    for raw_line in env_path.read_text().splitlines():
        line = raw_line.strip()
        if not line or line.startswith("#") or "=" not in line:
            continue
        key, value = line.split("=", 1)
        key = key.strip()
        value = value.strip().strip('"').strip("'")
        os.environ.setdefault(key, value)


def config_path() -> Path:
    env_path = os.environ.get("INTERNALVOICE_CONFIG")
    if env_path:
        configured = Path(env_path)
        if configured.exists():
            return configured
        logging.warning("INTERNALVOICE_CONFIG points to %s, but the file does not exist; falling back", configured)

    local = Path("config.toml")
    if local.exists():
        return local

    example = Path("config/InternalVoice.example.toml")
    if example.exists():
        return example

    home = Path.home()
    if sys.platform == "darwin":
        return home / "Library/Application Support/InternalVoice/config.toml"
    if sys.platform == "win32":
        appdata = Path(os.environ.get("APPDATA", home))
        return appdata / "InternalVoice/config.toml"
    return Path(os.environ.get("XDG_CONFIG_HOME", home / ".config")) / "InternalVoice/config.toml"


def data_dir() -> Path:
    env_path = os.environ.get("INTERNALVOICE_DATA_DIR")
    if env_path:
        return Path(env_path)

    local = Path(".internalvoice")
    if local.exists() or Path.cwd().exists():
        return local

    home = Path.home()
    if sys.platform == "darwin":
        return home / "Library/Application Support/InternalVoice"
    if sys.platform == "win32":
        appdata = Path(os.environ.get("LOCALAPPDATA", home))
        return appdata / "InternalVoice"
    return Path(os.environ.get("XDG_DATA_HOME", home / ".local/share")) / "InternalVoice"


def validate_config(config: AppConfig) -> None:
    if config.limits.requests_per_minute <= 0:
        raise InternalVoiceError("configuration error: limits.requests_per_minute must be > 0")
    if not config.service.gemini_model.strip():
        raise InternalVoiceError("configuration error: service.gemini_model cannot be empty")


class SecretProvider:
    def __init__(self, config: AppConfig) -> None:
        self.config = config

    def gemini_api_key(self) -> str:
        value = os.environ.get(self.config.secrets.env_var, "").strip()
        if value:
            return value

        value = self.config.service.gemini_api_key.strip()
        if value:
            return value

        keyring_error = None
        try:
            import keyring  # type: ignore

            value = keyring.get_password(
                self.config.secrets.keyring_service,
                self.config.secrets.keyring_account,
            )
            if value and value.strip():
                return value.strip()
            keyring_error = "No matching entry found in secure storage"
        except Exception as err:
            keyring_error = str(err)

        raise InternalVoiceError(
            "secret lookup failed: "
            f"Gemini API key not found. Tried env var `{self.config.secrets.env_var}`, "
            "config field `service.gemini_api_key`, "
            f"and keyring service `{self.config.secrets.keyring_service}` "
            f"account `{self.config.secrets.keyring_account}`; "
            f"keyring error: {keyring_error or 'keyring unavailable'}"
        )


class PromptCache:
    def __init__(self, path: Path, ttl_secs: int) -> None:
        self.path = path
        self.ttl_secs = ttl_secs
        self.conn = sqlite3.connect(path)
        self.conn.execute(
            """
            CREATE TABLE IF NOT EXISTS prompt_cache (
                key TEXT PRIMARY KEY,
                response TEXT NOT NULL,
                created_at INTEGER NOT NULL
            )
            """
        )
        self.conn.commit()

    def get(self, prompt: str) -> str | None:
        key = hashlib.sha256(prompt.encode("utf-8")).hexdigest()
        cutoff = int(time.time()) - self.ttl_secs
        row = self.conn.execute(
            "SELECT response FROM prompt_cache WHERE key = ? AND created_at >= ?",
            (key, cutoff),
        ).fetchone()
        return None if row is None else str(row[0])

    def put(self, prompt: str, response: str) -> None:
        key = hashlib.sha256(prompt.encode("utf-8")).hexdigest()
        self.conn.execute(
            """
            INSERT INTO prompt_cache(key, response, created_at)
            VALUES(?, ?, ?)
            ON CONFLICT(key) DO UPDATE SET
                response = excluded.response,
                created_at = excluded.created_at
            """,
            (key, response, int(time.time())),
        )
        self.conn.commit()


class StatePublisher:
    def __init__(self, current_state_path: Path) -> None:
        self.current_state_path = current_state_path

    def publish(self, state: dict[str, Any]) -> None:
        self.current_state_path.parent.mkdir(parents=True, exist_ok=True)
        self.current_state_path.write_text(json.dumps(state, indent=2))


class SystemSampler:
    def sample(self, max_processes: int) -> dict[str, Any]:
        now = iso_now()
        machine = {
            "hostname": socket.gethostname() or None,
            "os_name": platform.system() or None,
            "kernel_version": platform.release() or None,
            "uptime_secs": system_uptime_secs(),
        }

        cpu_percent = cpu_usage_percent()
        per_core = per_core_usage_percent()
        memory = memory_state()
        storage = storage_state()
        battery = battery_state()
        processes = top_processes(max_processes)
        alerts = platform_alerts()

        return {
            "collected_at": now,
            "machine": machine,
            "cpu": {
                "total_usage_percent": cpu_percent,
                "per_core_usage_percent": per_core,
                "temperature_celsius": None,
            },
            "memory": memory,
            "storage": storage,
            "battery": battery,
            "processes": processes,
            "alerts": alerts,
            "container_runtime": None,
            "network": None,
            "peripheral_devices": None,
        }


class SystemStateService:
    def __init__(self) -> None:
        self.sampler = SystemSampler()
        self.last_telemetry_sample: float | None = None

    def sample(self, config: AppConfig) -> dict[str, Any]:
        now = time.monotonic()
        should_collect_full = (
            self.last_telemetry_sample is None
            or (now - self.last_telemetry_sample) >= config.system_telemetry_interval()
        )
        if should_collect_full:
            logging.debug("Collecting full system telemetry")
            self.last_telemetry_sample = now

        state = self.sampler.sample(config.narration.max_processes)
        state["alerts"].extend(evaluate_alerts(state, config))
        return state


def evaluate_alerts(state: dict[str, Any], config: AppConfig) -> list[dict[str, str]]:
    alerts: list[dict[str, str]] = []
    cpu = float(state["cpu"]["total_usage_percent"])
    memory_pct = float(state["memory"]["used_percent"])

    if cpu >= config.policy.notify_on_cpu_above:
        alerts.append(alert("Warning", "cpu.high", f"CPU usage is {cpu:.1f}%"))

    if memory_pct >= config.policy.notify_on_memory_above:
        alerts.append(alert("Warning", "memory.high", f"Memory usage is {memory_pct:.1f}%"))

    for volume in state["storage"]:
        free_percent = 100.0 - float(volume["used_percent"])
        if free_percent <= config.policy.notify_on_disk_below:
            alerts.append(
                alert(
                    "Warning",
                    "disk.low",
                    f"{volume['name']} has only {free_percent:.1f}% free space remaining",
                )
            )

    battery = state.get("battery")
    if battery and not battery["on_ac_power"] and float(battery["percent"]) <= config.policy.notify_on_battery_below:
        alerts.append(
            alert("Critical", "battery.low", f"Battery is at {float(battery['percent']):.1f}%")
        )

    return alerts


def alert(severity: str, code: str, message: str) -> dict[str, str]:
    return {"severity": severity, "code": code, "message": message}


@dataclasses.dataclass
class SpokenMessage:
    timestamp: str
    utterance: str


class PolicyEngine:
    def __init__(self) -> None:
        self.last_announcement_monotonic: float | None = None
        self.state_history: deque[dict[str, Any]] = deque(maxlen=10)
        self.message_history: deque[SpokenMessage] = deque(maxlen=5)

    def record_spoken(self, utterance: str) -> None:
        self.message_history.append(SpokenMessage(timestamp=iso_now(), utterance=utterance))

    def system_instructions(self, config: AppConfig) -> str:
        return "\n".join(
            [
                "You are a system monitor narrator.",
                f"Your style is {config.narration.style}.",
                "Decide if the user needs to be interrupted.",
                "Summarize only actionable system conditions.",
                "Do not suggest shell commands.",
                "Produce a short, spoken-friendly summary (1–2 sentences).",
                "Optionally suggest a single action (e.g., close app X, plug in power, free disk).",
                "Avoid repeating the same warning unless the situation worsens significantly.",
            ]
        )

    def build_prompt(self, state: dict[str, Any], config: AppConfig) -> dict[str, Any] | None:
        self.state_history.append(state)
        if not state["alerts"] and not is_resource_pressure(state, config):
            return None

        now = time.monotonic()
        if self.last_announcement_monotonic is not None:
            if (now - self.last_announcement_monotonic) < config.announcement_interval():
                return None

        self.last_announcement_monotonic = now

        prompt = {
            "role": "system monitor narrator",
            "style": config.narration.style,
            "goal": "Decide if the user needs to be interrupted.",
            "instructions": [
                "Summarize only actionable system conditions.",
                "Do not suggest shell commands.",
                "Produce a short, spoken-friendly summary (1–2 sentences).",
                "Optionally suggest a single action (e.g., close app X, plug in power, free disk).",
                "Avoid repeating the same warning unless the situation worsens significantly.",
                "Output your decision in RAW JSON format. No markdown blocks.",
            ],
            "policies": {
                "telemetry": config.policy.system_telemetry,
                "thresholds": {
                    "cpu_above": config.policy.notify_on_cpu_above,
                    "memory_above": config.policy.notify_on_memory_above,
                    "battery_below": config.policy.notify_on_battery_below,
                    "disk_below": config.policy.notify_on_disk_below,
                },
            },
            "context": {
                "latest_state": state,
                "recent_history": list(self.state_history)[:-1],
                "last_messages_spoken": [dataclasses.asdict(message) for message in self.message_history],
            },
            "output_format": {
                "speak": "boolean",
                "utterance": "string | null",
                "priority": "low|medium|high | null",
            },
        }

        return {
            "cache_key": f"{state['collected_at']}:{len(state['alerts'])}:{len(self.message_history)}",
            "prompt": json.dumps(prompt),
            "should_speak": config.narration.voice_enabled,
        }


def is_resource_pressure(state: dict[str, Any], config: AppConfig) -> bool:
    battery = state.get("battery")
    return (
        float(state["cpu"]["total_usage_percent"]) >= config.policy.notify_on_cpu_above
        or float(state["memory"]["used_percent"]) >= config.policy.notify_on_memory_above
        or (battery is not None and float(battery["percent"]) <= config.policy.notify_on_battery_below)
        or any(item["severity"] == "Critical" for item in state["alerts"])
    )


class GeminiClient:
    def __init__(self, api_key: str, config: AppConfig) -> None:
        self.api_key = api_key
        self.model = config.service.gemini_model
        self.timeout = config.gemini_timeout()
        self.breaker_failure_threshold = config.limits.breaker_failure_threshold
        self.breaker_reset_secs = config.breaker_reset()
        self.debounce_interval_secs = config.debounce_interval()
        self.requests_per_minute = config.limits.requests_per_minute
        self.request_times: deque[float] = deque()
        self.consecutive_failures = 0
        self.breaker_open_until: float | None = None
        self.session = requests.Session()

    def generate_summary(self, prompt: str, max_output_tokens: int) -> str:
        self.await_turn()
        self.ensure_breaker_closed()

        url = (
            f"https://generativelanguage.googleapis.com/v1beta/models/"
            f"{self.model}:streamGenerateContent?alt=sse&key={self.api_key}"
        )
        request = {
            "contents": [{"role": "user", "parts": [{"text": prompt}]}],
            "generation_config": {
                "temperature": 0.3,
                "max_output_tokens": max_output_tokens,
            },
        }

        try:
            with self.session.post(url, json=request, timeout=self.timeout, stream=True) as response:
                if response.status_code >= 500:
                    self.record_failure()
                    raise InternalVoiceError(f"gemini api error: server returned {response.status_code}")

                if response.status_code != 200:
                    body = response.text
                    self.record_failure()
                    raise InternalVoiceError(
                        f"gemini api error: request failed with {response.status_code} {response.reason}: {body}"
                    )

                buffer: list[str] = []
                started = time.monotonic()

                for raw_line in response.iter_lines(decode_unicode=True):
                    if raw_line is None:
                        continue
                    line = raw_line.strip()
                    if not line.startswith("data: "):
                        continue
                    data = line[6:]
                    if data == "[DONE]":
                        self.record_success()
                        return "".join(buffer).strip()

                    parsed = json.loads(data)
                    for candidate in parsed.get("candidates", []):
                        content = candidate.get("content") or {}
                        for part in content.get("parts", []):
                            text = part.get("text")
                            if text:
                                buffer.append(text)

                    if (time.monotonic() - started) > self.timeout:
                        self.record_failure()
                        raise InternalVoiceError("gemini api error: stream timed out")

                self.record_success()
                return "".join(buffer).strip()
        except requests.RequestException as err:
            self.record_failure()
            raise InternalVoiceError(f"network error: {err}") from err

    def await_turn(self) -> None:
        now = time.monotonic()
        while self.request_times and (now - self.request_times[0]) >= 60.0:
            self.request_times.popleft()
        if len(self.request_times) >= self.requests_per_minute:
            sleep_for = 60.0 - (now - self.request_times[0])
            if sleep_for > 0:
                time.sleep(sleep_for)
        time.sleep(self.debounce_interval_secs)
        self.request_times.append(time.monotonic())

    def ensure_breaker_closed(self) -> None:
        if self.breaker_open_until is None:
            return
        if time.monotonic() < self.breaker_open_until:
            raise InternalVoiceError("gemini api error: circuit breaker is open after repeated failures")
        self.breaker_open_until = None
        self.consecutive_failures = 0

    def record_success(self) -> None:
        self.consecutive_failures = 0
        self.breaker_open_until = None

    def record_failure(self) -> None:
        self.consecutive_failures += 1
        if self.consecutive_failures >= self.breaker_failure_threshold:
            self.breaker_open_until = time.monotonic() + self.breaker_reset_secs


class ServiceContext:
    def __init__(self, config: AppConfig) -> None:
        self.config = config
        data = data_dir()
        data.mkdir(parents=True, exist_ok=True)

        api_key = SecretProvider(config).gemini_api_key()
        self.gemini = GeminiClient(api_key, config)
        self.cache = PromptCache(data / "prompt-cache.sqlite3", config.service.cache_ttl_secs)
        self.state_service = SystemStateService()
        self.policy = PolicyEngine()
        self.publisher = StatePublisher(data / "current-state.json")


def validate_action(action: str) -> bool:
    return action == "Announce"


def init_logging(config: AppConfig) -> None:
    log_dir = config.log_dir_path()
    log_dir.mkdir(parents=True, exist_ok=True)
    log_path = log_dir / "internalvoice.log"

    root = logging.getLogger()
    root.setLevel(logging.INFO)
    root.handlers.clear()

    formatter = logging.Formatter("%(asctime)s %(levelname)s %(message)s")

    stdout_handler = logging.StreamHandler()
    stdout_handler.setFormatter(formatter)
    root.addHandler(stdout_handler)

    file_handler = logging.handlers.TimedRotatingFileHandler(
        log_path,
        when="midnight",
        backupCount=config.service.max_log_files,
        encoding="utf-8",
    )
    file_handler.setFormatter(formatter)
    root.addHandler(file_handler)

    logging.info(
        "log system initialized",
        extra={
            "log_dir": str(log_dir),
            "max_files": config.service.max_log_files,
            "max_size_mb": config.service.max_log_file_size_mb,
        },
    )


def enforce_least_privilege() -> None:
    if hasattr(os, "geteuid") and os.geteuid() == 0:
        logging.warning("service is running as root; deploy with a restricted service user")


def run_polling_service(context: ServiceContext, stop_event: threading.Event) -> None:
    while not stop_event.is_set():
        state = context.state_service.sample(context.config)
        context.publisher.publish(state)

        prompt = context.policy.build_prompt(state, context.config)
        if prompt is not None:
            if not validate_action("Announce"):
                logging.warning("announcement action was rejected by allow-list")
            else:
                cached = context.cache.get(prompt["prompt"])
                if cached:
                    logging.info("cache hit")
                    handle_decision(context, cached, cached_response=True)
                else:
                    try:
                        raw_json = context.gemini.generate_summary(
                            prompt["prompt"],
                            context.config.service.max_prompt_tokens_hint,
                        )
                        handle_decision(context, raw_json, cached_response=False)
                        context.cache.put(prompt["prompt"], raw_json)
                    except InternalVoiceError as err:
                        logging.warning("Gemini request failed error=%s", err)
                        speak_alerts_fallback(context, state)

        stop_event.wait(context.config.sample_interval())


def speak_utterance(text: str) -> None:
    def _speak() -> None:
        try:
            if sys.platform == "darwin":
                subprocess.run(["say", text], check=False, timeout=30)
            elif sys.platform == "win32":
                safe = text.replace('"', "'")
                subprocess.run(
                    [
                        "powershell",
                        "-Command",
                        f'Add-Type -AssemblyName System.Speech; (New-Object System.Speech.Synthesis.SpeechSynthesizer).Speak("{safe}")',
                    ],
                    check=False,
                    timeout=30,
                )
            else:
                if shutil.which("espeak"):
                    subprocess.run(["espeak", text], check=False, timeout=30)
                elif shutil.which("festival"):
                    subprocess.run(["festival", "--tts"], input=text.encode(), check=False, timeout=30)
                else:
                    logging.warning("tts: no speech engine found (espeak/festival); skipping")
        except Exception as err:
            logging.warning("tts speech failed: %s", err)

    threading.Thread(target=_speak, daemon=True).start()


def handle_decision(context: ServiceContext, raw_json: str, cached_response: bool) -> None:
    try:
        decision = json.loads(raw_json)
    except json.JSONDecodeError as err:
        logging.warning("Failed to parse interruption decision error=%s", err)
        return

    if not decision.get("speak"):
        return

    utterance = decision.get("utterance")
    if not utterance:
        return

    priority = decision.get("priority")
    if cached_response:
        logging.info("Speaking (cached) text=%s priority=%s", utterance, priority)
    else:
        logging.info("Gemini said text=%s priority=%s", utterance, priority)

    context.policy.record_spoken(str(utterance))

    if context.config.narration.voice_enabled:
        speak_utterance(str(utterance))


def speak_alerts_fallback(context: ServiceContext, state: dict[str, Any]) -> None:
    """Announce current alerts via local TTS when the Gemini backend is unreachable."""
    if not context.config.narration.voice_enabled:
        return

    alerts = state.get("alerts", [])
    if not alerts:
        return

    critical = [a for a in alerts if a.get("severity") == "Critical"]
    warnings = [a for a in alerts if a.get("severity") == "Warning"]
    selected = (critical if critical else warnings)[:2]
    if not selected:
        return

    utterance = ". ".join(a["message"] for a in selected)
    remainder = len(alerts) - len(selected)
    if remainder > 0:
        utterance += f". Plus {remainder} additional alert{'s' if remainder > 1 else ''}."

    logging.info("Fallback TTS (Gemini unavailable) text=%s", utterance)
    context.policy.record_spoken(utterance)
    speak_utterance(utterance)


def iso_now() -> str:
    return dt.datetime.now(dt.UTC).isoformat().replace("+00:00", "Z")


def system_uptime_secs() -> int:
    if psutil is not None:
        try:
            return int(time.time() - psutil.boot_time())
        except Exception:
            pass

    if sys.platform == "darwin":
        try:
            output = subprocess.check_output(
                ["sysctl", "-n", "kern.boottime"],
                text=True,
                stderr=subprocess.DEVNULL,
            ).strip()
            sec_part = output.split("sec = ", 1)[1].split(",", 1)[0]
            return max(0, int(time.time()) - int(sec_part))
        except Exception:
            return 0

    if Path("/proc/uptime").exists():
        try:
            return int(float(Path("/proc/uptime").read_text().split()[0]))
        except Exception:
            return 0

    return 0


def cpu_usage_percent() -> float:
    if psutil is not None:
        try:
            return float(psutil.cpu_percent(interval=None))
        except Exception:
            pass

    if hasattr(os, "getloadavg"):
        load1 = os.getloadavg()[0]
        cpu_count = os.cpu_count() or 1
        return max(0.0, min(100.0, (load1 / cpu_count) * 100.0))
    return 0.0


def per_core_usage_percent() -> list[float]:
    if psutil is not None:
        try:
            return [float(value) for value in psutil.cpu_percent(interval=None, percpu=True)]
        except Exception:
            pass

    count = os.cpu_count() or 1
    return [cpu_usage_percent() for _ in range(count)]


def memory_state() -> dict[str, Any]:
    if psutil is not None:
        vm = psutil.virtual_memory()
        swap = psutil.swap_memory()
        return {
            "total_bytes": int(vm.total),
            "used_bytes": int(vm.used),
            "swap_total_bytes": int(swap.total),
            "swap_used_bytes": int(swap.used),
            "used_percent": float(vm.percent),
        }

    total = 0
    available = 0
    if hasattr(os, "sysconf"):
        try:
            page_size = int(os.sysconf("SC_PAGE_SIZE"))
            total = int(os.sysconf("SC_PHYS_PAGES")) * page_size
            available = int(os.sysconf("SC_AVPHYS_PAGES")) * page_size
        except (ValueError, OSError):
            total = 0
            available = 0

    used = max(0, total - available)
    used_percent = 0.0 if total == 0 else (used / total) * 100.0
    return {
        "total_bytes": total,
        "used_bytes": used,
        "swap_total_bytes": 0,
        "swap_used_bytes": 0,
        "used_percent": used_percent,
    }


def storage_state() -> list[dict[str, Any]]:
    volumes: list[dict[str, Any]] = []

    if psutil is not None:
        try:
            seen: set[str] = set()
            for part in psutil.disk_partitions(all=False):
                if part.mountpoint in seen:
                    continue
                seen.add(part.mountpoint)
                try:
                    usage = psutil.disk_usage(part.mountpoint)
                except Exception:
                    continue
                volumes.append(
                    {
                        "name": part.device or part.mountpoint,
                        "mount_point": part.mountpoint,
                        "total_bytes": int(usage.total),
                        "available_bytes": int(usage.free),
                        "used_percent": float(usage.percent),
                    }
                )
            return volumes
        except Exception:
            volumes = []

    usage = shutil.disk_usage("/")
    volumes.append(
        {
            "name": "/",
            "mount_point": "/",
            "total_bytes": int(usage.total),
            "available_bytes": int(usage.free),
            "used_percent": 0.0 if usage.total == 0 else ((usage.used / usage.total) * 100.0),
        }
    )
    return volumes


def battery_state() -> dict[str, Any] | None:
    if psutil is None:
        return None
    try:
        battery = psutil.sensors_battery()
    except Exception:
        return None
    if battery is None:
        return None
    return {
        "percent": float(battery.percent),
        "on_ac_power": bool(battery.power_plugged),
    }


def top_processes(max_processes: int) -> list[dict[str, Any]]:
    if psutil is None:
        return []

    processes: list[dict[str, Any]] = []
    try:
        for process in psutil.process_iter(["pid", "name", "cpu_percent", "memory_info", "status"]):
            info = process.info
            memory_info = info.get("memory_info")
            processes.append(
                {
                    "pid": str(info.get("pid", "")),
                    "name": str(info.get("name") or ""),
                    "cpu_percent": float(info.get("cpu_percent") or 0.0),
                    "memory_bytes": int(getattr(memory_info, "rss", 0)),
                    "status": str(info.get("status") or ""),
                }
            )
    except Exception:
        return []

    processes.sort(key=lambda item: (item["cpu_percent"], item["memory_bytes"]), reverse=True)
    return processes[:max_processes]


def platform_alerts() -> list[dict[str, str]]:
    return []


def install_signal_handlers(stop_event: threading.Event) -> None:
    def stop_handler(signum: int, _frame: Any) -> None:
        logging.info("shutdown signal received signal=%s", signum)
        stop_event.set()

    signal.signal(signal.SIGINT, stop_handler)
    if hasattr(signal, "SIGTERM"):
        signal.signal(signal.SIGTERM, stop_handler)


def main() -> int:
    load_dotenv()

    try:
        config = AppConfig.load()
        validate_config(config)
        init_logging(config)
        enforce_least_privilege()

        context = ServiceContext(config)
        stop_event = threading.Event()
        install_signal_handlers(stop_event)

        logging.info("InternalVoice starting")
        run_polling_service(context, stop_event)
        logging.info("InternalVoice stopped")
        return 0
    except InternalVoiceError as err:
        print(f"Error: {err}", file=sys.stderr)
        return 1
    except KeyboardInterrupt:
        return 130


if __name__ == "__main__":
    raise SystemExit(main())
