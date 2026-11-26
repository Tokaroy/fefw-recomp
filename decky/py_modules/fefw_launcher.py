# SPDX-License-Identifier: GPL-3.0-or-later
"""Drives fefw-launcher.AppImage's commands without its window (launcher/README.md) for the plugin. Nothing here needs Decky, so the
tests run anywhere. All the update and build logic stays in the launcher."""

import asyncio
import json
import os
import time
from collections import deque
from pathlib import Path

APPIMAGE = "fefw-launcher.AppImage"
API = 1
TOO_OLD = ("This install's launcher cannot be driven by the plugin. In Desktop Mode, open the launcher and update to a newer release.")
NEW_TOO_OLD = "The new release's launcher cannot be driven by the plugin: build the game in the launcher, in Desktop Mode."
# Variables of the plugin loader's own Python that would break the programs the launcher starts.
LOADER_VARIABLES = ("LD_LIBRARY_PATH", "LD_PRELOAD", "PYTHONHOME", "PYTHONPATH", "XDG_CONFIG_HOME")
DISPLAY_VARIABLES = ("DISPLAY", "WAYLAND_DISPLAY", "XAUTHORITY", "GAMESCOPE_WAYLAND_DISPLAY")
# For the questions: with WAYLAND_DISPLAY unset a window toolkit still tries the session's wayland-0, so a name that is not there.
NO_DISPLAY = {"WAYLAND_DISPLAY": "fefw-plugin-no-display"}
LINE_LIMIT = 4 * 1024 * 1024  # the release notes come as one line
QUERY_TIMEOUT = 120.0
# The shader preparation runs the game. A display that does not work can leave it waiting for ever, with nothing said.
PREPARE_TIMEOUT = 600.0
POLL = 1.0  # how often a silent launcher is looked at


class LauncherError(Exception):
    pass


def is_install(folder):
    """A release package's folder."""
    folder = Path(folder)
    return (folder / APPIMAGE).is_file() and (folder / "tools" / "build" / "fefw_build.py").is_file()


def find_display(proc="/proc", uid=None):
    """The display variables of the user's running Steam client, which has the session's: the loader runs without one. {} when
    there is none to be found."""
    uid = os.getuid() if uid is None else uid
    for entry in sorted(Path(proc).glob("[0-9]*"), key=lambda p: int(p.name)):
        try:
            if entry.stat().st_uid != uid or (entry / "comm").read_text().strip() != "steam":
                continue
            raw = (entry / "environ").read_bytes()
        except OSError:
            continue
        env = dict(item.split("=", 1) for item in raw.decode("utf-8", "replace").split("\0") if "=" in item)
        found = {k: env[k] for k in DISPLAY_VARIABLES if env.get(k)}
        if "DISPLAY" in found or "WAYLAND_DISPLAY" in found:
            return found
    return {}


def environment(base, home, display=None, run_dir=None):
    """The environment the launcher gets: the user's, without the loader's own variables, and with no display unless one is given."""
    env = {k: v for k, v in base.items() if k not in LOADER_VARIABLES and k not in DISPLAY_VARIABLES}
    env["HOME"] = str(home)
    env.setdefault("PATH", "/usr/local/bin:/usr/bin:/bin")
    run_dir = Path(run_dir or f"/run/user/{os.getuid()}")
    if run_dir.is_dir():  # the Steam runtime looks for the user's session here
        env.setdefault("XDG_RUNTIME_DIR", str(run_dir))
        env.setdefault("DBUS_SESSION_BUS_ADDRESS", f"unix:path={run_dir}/bus")
    env.update(display or {})
    return env


class Command:
    """One run of a launcher command."""

    def __init__(self, install, argument, env):
        self.install, self.argument, self.env = Path(install), argument, env
        self.process = None
        self.timed_out = False
        self._cancelled = False

    async def run(self, on_progress=None, timeout=None, prepare_timeout=PREPARE_TIMEOUT):
        """The command's end line. LauncherError when it gives none."""
        try:
            self.process = await asyncio.create_subprocess_exec(
                str(self.install / APPIMAGE), self.argument, cwd=str(self.install), env=self.env, limit=LINE_LIMIT,
                stdin=asyncio.subprocess.PIPE, stdout=asyncio.subprocess.PIPE, stderr=asyncio.subprocess.PIPE,
                start_new_session=True)
        except OSError as e:
            raise LauncherError(f"Cannot start the launcher: {e}") from e
        if self._cancelled:
            self.cancel()
        errors = deque(maxlen=5)
        drain = asyncio.ensure_future(self._drain(errors))
        try:
            return await self._answer(drain, errors, on_progress, timeout, prepare_timeout)
        finally:
            drain.cancel()  # something the launcher started may keep its output open: not waited for
            transport = getattr(self.process, "_transport", None)
            if transport and self.process.returncode is not None:
                transport.close()

    async def _answer(self, drain, errors, on_progress, timeout, prepare_timeout):
        try:
            end = await asyncio.wait_for(self._read(on_progress, prepare_timeout), timeout)
        except asyncio.TimeoutError:
            await self.stop()
            raise LauncherError("The launcher did not answer.") from None
        except ValueError as e:  # a line over the limit, or another interface version
            await self.stop()
            raise LauncherError(f"The launcher sent an answer that cannot be read: {e}") from e
        except LauncherError:
            await self.stop()
            raise
        try:
            await asyncio.wait_for(self.process.wait(), 10)
        except asyncio.TimeoutError:
            await self.stop()
        if end is None:
            await asyncio.wait([drain], timeout=0.2)  # its last words
            detail = " ".join(errors)
            raise LauncherError(f"The launcher stopped without an answer. {detail}".strip())
        return end

    async def _drain(self, errors):
        while True:
            line = await self.process.stderr.readline()
            if not line:
                return
            errors.append(line.decode("utf-8", "replace").strip())

    async def _read(self, on_progress, prepare_timeout):
        """The end line, or None when the launcher ended without one."""
        phase, silent, poll = None, 0.0, min(POLL, prepare_timeout)
        while True:
            try:
                raw = await asyncio.wait_for(self.process.stdout.readline(), poll)
            except asyncio.TimeoutError:
                silent += poll
                if self.process.returncode is not None:  # gone, though something it started may hold its output open
                    return None
                if phase == "prepare" and silent >= prepare_timeout:
                    if self.timed_out:
                        raise LauncherError("Preparing the shaders took too long, and the launcher did not stop.")
                    self.timed_out, silent = True, 0.0
                    self.cancel()
                continue
            silent = 0.0
            if not raw:
                return None
            try:
                line = json.loads(raw)
                event = line["event"]
            except (ValueError, KeyError, TypeError):
                continue  # not one of the interface's lines
            if line.get("api") != API:
                raise ValueError(f"interface version {line.get('api')}, the plugin knows {API}")
            if event == "end":
                return line
            if event == "progress":
                phase = line.get("phase")
                if on_progress:
                    on_progress(line)

    def cancel(self):
        """Asks the launcher to stop: it ends what it started itself."""
        self._cancelled = True
        if self.process and self.process.returncode is None:
            try:
                self.process.stdin.write(b"cancel\n")
            except (OSError, RuntimeError):
                pass

    async def stop(self, grace=3.0):
        """Cancel, and end the launcher if it does not stop."""
        self.cancel()
        if not self.process or self.process.returncode is not None:
            return
        try:
            await asyncio.wait_for(self.process.wait(), grace)
        except asyncio.TimeoutError:
            try:
                os.killpg(self.process.pid, 9)
            except OSError:
                pass


def _timers(value):
    """{"battery": seconds, "ac": seconds}, or None for anything else."""
    if isinstance(value, dict) and all(type(value.get(k)) is int and 0 <= value[k] < 2**31 for k in ("battery", "ac")):
        return {"battery": value["battery"], "ac": value["ac"]}
    return None


class Controller:
    """What the panel shows and does: the chosen install, its status, and at most one job (an update or a rebuild)."""

    def __init__(self, settings_file, home, base_env=None, display=find_display, on_done=None):
        self.settings_file = Path(settings_file)
        self.home = Path(home)
        self.base_env = dict(os.environ if base_env is None else base_env)
        self.display = display
        self.on_done = on_done
        saved = self._load()
        self.install_dir = saved.get("install_dir") or None
        self.keep_awake = saved.get("keep_awake") is not False  # on unless it was turned off
        # Steam's auto-sleep timers as they were before the panel turned them off for a job (src/sleep.ts), until put back
        self.held_sleep = _timers(saved.get("held_sleep"))
        self.status = None
        self.update = None
        self.job = None
        self.result = None
        self.error = None
        self.prepare_timeout = PREPARE_TIMEOUT
        self._command = None
        self._task = None
        self._cancel = False
        self._started = 0.0

    def _load(self):
        try:
            saved = json.loads(self.settings_file.read_text())
        except (OSError, ValueError):
            return {}
        return saved if isinstance(saved, dict) else {}

    def _save(self):
        self.settings_file.parent.mkdir(parents=True, exist_ok=True)
        saved = {"install_dir": self.install_dir, "keep_awake": self.keep_awake, "held_sleep": self.held_sleep}
        # whole or not at all: held_sleep is the only way back to the user's timers
        part = self.settings_file.with_suffix(".part")
        part.write_text(json.dumps(saved))
        part.replace(self.settings_file)

    def set_keep_awake(self, on):
        self.keep_awake = bool(on)
        self._save()

    def hold_sleep(self, battery, ac):
        """Records the timers about to be turned off. The first record stays until released: later ones would read "never"."""
        if self.held_sleep is None:
            timers = _timers({"battery": battery, "ac": ac})
            if timers is None:
                raise LauncherError("Those are not sleep timers.")
            self.held_sleep = timers
            self._save()

    def release_sleep(self):
        self.held_sleep = None
        self._save()

    def state(self):
        # counted here, so that it is right when the panel is opened again
        job = self.job and dict(self.job, elapsed=int(time.monotonic() - self._started))
        return {"install_dir": self.install_dir, "home": str(self.home), "status": self.status, "update": self.update,
                "job": job, "result": self.result, "error": self.error,
                "keep_awake": self.keep_awake, "held_sleep": self.held_sleep}

    def set_install(self, folder):
        if self.job:
            raise LauncherError("Wait for the running job to finish.")
        if not is_install(folder):
            raise LauncherError("That folder is not the game's: choose the one with fefw-launcher.AppImage in it.")
        self.install_dir = str(folder)
        self._save()
        self.status = self.update = self.result = self.error = None

    def _new(self, argument, display=None):
        if not self.install_dir or not is_install(self.install_dir):
            raise LauncherError("Choose the game's folder first.")
        env = environment(self.base_env, self.home, display)
        return Command(self.install_dir, argument, env)

    async def refresh(self):
        """Reads the install's status and asks for a newer release. Problems end up in `error`. Not during a job."""
        if not self.job:
            await self._look()

    async def _look(self):
        self.error = None
        folder = self.install_dir
        try:
            # with no display: a launcher too old for these commands then cannot open its window, and ends instead
            try:
                status = await self._new("--status", NO_DISPLAY).run(timeout=QUERY_TIMEOUT)
            except LauncherError as e:
                self._forget(folder)
                raise LauncherError(f"{TOO_OLD} ({e})") from e
            if status.get("state") != "done":
                self._forget(folder)
                raise LauncherError(status.get("message") or "The status could not be read.")
            if folder != self.install_dir:
                return
            self.status = status
            try:
                check = await self._new("--check-update", NO_DISPLAY).run(timeout=QUERY_TIMEOUT)
                if check.get("state") != "done":
                    raise LauncherError(check.get("message") or "The update check failed.")
            except LauncherError:
                if folder == self.install_dir:
                    self.update = None  # what an earlier check said may no longer hold
                raise
            if folder == self.install_dir:
                self.update = check
        except LauncherError as e:
            self.error = str(e)

    def _forget(self, folder):
        """What was read before is not shown for an install that can no longer be read: it may have changed."""
        if folder == self.install_dir:
            self.status = self.update = None

    def start(self, kind):
        """Starts "update" (which goes on to rebuild when the new release needs it) or "rebuild"."""
        if kind not in ("update", "rebuild"):
            raise LauncherError(f"Unknown job: {kind}")
        if self.job:
            raise LauncherError("A job is already running.")
        if not self.status:
            raise LauncherError("The install's status has not been read yet.")
        self._cancel = False
        self.result = self.error = None
        self._started = time.monotonic()
        self.job = {"kind": kind, "step": kind, "phase": None, "message": "Starting", "fraction": None, "cancelling": False}
        self._task = asyncio.ensure_future(self._job(kind))

    async def _job(self, kind):
        try:
            result = await self._steps(kind)
        except LauncherError as e:
            result = {"state": "error", "message": str(e)}
        except Exception as e:  # the panel must not be left with a job that never ends
            result = {"state": "error", "message": f"The plugin failed: {e!r}"}
        self._command = None
        await self._look()  # still the job's: another cannot start before this one's result is there
        if result["message"].endswith(NEW_TOO_OLD):
            self.error = None  # the result says it, without advising the update that was just made
        self.result = result
        self.job = None
        if self.on_done:
            await self.on_done(result)

    async def _steps(self, kind):
        if kind == "update":
            end = await self._step("update", "--update")
            if end["state"] != "done" or self._cancel:
                return self._outcome(end)
            if end.get("rebuild") is False:
                return {"state": "done", "message": end.get("message") or "Updated."}
            # a launcher without the commands would ignore --rebuild and, with a display, open its window
            if not self._cancel and not await self._driveable():
                return {"state": "error", "message": f"{end.get('message') or 'Updated.'} {NEW_TOO_OLD}"}
        # the new launcher's, after an update
        return self._outcome(await self._step("rebuild", "--rebuild", self.display()))

    async def _driveable(self):
        """Whether the install's launcher answers --status, asked with no display."""
        try:
            self._command = self._new("--status", NO_DISPLAY)
            return (await self._command.run(timeout=QUERY_TIMEOUT)).get("state") == "done"
        except LauncherError:
            return False

    async def _step(self, step, argument, display=None):
        if self._cancel:
            return {"state": "cancelled"}
        self._command = command = self._new(argument, display)
        self.job.update(step=step, phase=None, message="Starting", fraction=None)

        def progress(line):
            done, total = line.get("done") or 0, line.get("total") or 0
            fraction = line.get("fraction")
            if fraction is None and total:
                fraction = min(done / total, 1.0)
            self.job.update(phase=line.get("phase"), message=line.get("message") or "", fraction=fraction)

        end = await command.run(progress, prepare_timeout=self.prepare_timeout)
        if command.timed_out and end["state"] == "cancelled" and not self._cancel:
            # the game is built; only the preparation, which saves compile time while playing, was stopped
            return {"state": "done", "message": "Ready to play. Preparing the shaders took too long and was stopped."}
        return end

    def _outcome(self, end):
        if end["state"] == "cancelled":
            return {"state": "cancelled", "message": "Cancelled."}
        return {"state": end["state"], "message": end.get("message") or ""}

    def cancel(self):
        if not self.job:
            return
        self._cancel = True
        self.job["cancelling"] = True
        if self._command:
            self._command.cancel()

    async def close(self):
        """The plugin is being unloaded: nothing the launcher started may be left behind. The loader allows five seconds."""
        self._cancel = True
        if self._command:
            await self._command.stop()
        if self._task:
            self._task.cancel()
