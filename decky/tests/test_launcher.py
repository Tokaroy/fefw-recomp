# SPDX-License-Identifier: GPL-3.0-or-later
"""The plugin's backend against a stand-in launcher: python3 -m unittest discover -s tests"""

import asyncio
import json
import os
import sys
import tempfile
import time
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent / "py_modules"))
import fefw_launcher as fl  # noqa: E402

# What each command does comes from plan.json beside it; what it was run with goes to runs.jsonl.
FAKE = r'''#!/usr/bin/env python3
import json, os, subprocess, sys, time
here = os.path.dirname(os.path.abspath(__file__))
argument = sys.argv[1]
with open(os.path.join(here, "runs.jsonl"), "a") as f:
    f.write(json.dumps({"argument": argument, "env": dict(os.environ), "pid": os.getpid()}) + "\n")
def out(event, **fields):
    print(json.dumps(dict(fields, event=event, api=fields.pop("api", 1))), flush=True)
for do, value in json.load(open(os.path.join(here, "plan.json")))[argument]:
    if do == "progress": out("progress", **value)
    elif do == "end": out("end", **value); sys.exit(0 if value["state"] == "done" else 1)
    elif do == "text": print(value, flush=True)
    elif do == "sleep": time.sleep(value)
    elif do == "await-cancel":
        for line in sys.stdin:
            if line.strip() == "cancel": break
        out("end", state="cancelled", message="Paused."); sys.exit(1)
    elif do == "child": subprocess.Popen(["sleep", str(value)])
    elif do == "plan": json.dump(value, open(os.path.join(here, "plan.json"), "w"))  # as an update replaces the launcher
    elif do == "die": print(value, file=sys.stderr, flush=True); sys.exit(101)
'''

STATUS = {"state": "done", "version": "1.0.0", "busy": None, "setup_complete": True, "rebuild": False, "rebuild_reason": "",
          "problem": None}
CHECK = {"state": "done", "newer": True, "latest": {"version": "1.1.0", "notes": "n" * 200_000}}


def plan(**commands):
    steps = {"--status": [["end", STATUS]], "--check-update": [["end", CHECK]]}
    steps.update({"--" + k.replace("_", "-"): v for k, v in commands.items()})
    return steps


class Case(unittest.IsolatedAsyncioTestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.root = Path(self.tmp.name)
        self.install = self.root / "game"
        (self.install / "tools" / "build").mkdir(parents=True)
        (self.install / "tools" / "build" / "fefw_build.py").touch()
        launcher = self.install / fl.APPIMAGE
        launcher.write_text(FAKE)
        launcher.chmod(0o755)
        self.done = []

        async def on_done(result):
            self.done.append(result)

        base = {"PATH": os.environ["PATH"], "LD_LIBRARY_PATH": "/loader", "DISPLAY": ":9", "KEPT": "yes"}
        self.controller = fl.Controller(self.root / "settings" / "settings.json", self.root, base_env=base,
                                        display=lambda: {"DISPLAY": ":1"}, on_done=on_done)

    def tearDown(self):
        self.tmp.cleanup()

    def write_plan(self, **commands):
        (self.install / "plan.json").write_text(json.dumps(plan(**commands)))

    def runs(self):
        return [json.loads(line) for line in (self.install / "runs.jsonl").read_text().splitlines()]

    async def ready(self, **commands):
        self.write_plan(**commands)
        self.controller.set_install(str(self.install))
        await self.controller.refresh()
        self.assertIsNone(self.controller.error)

    async def finish(self):
        await asyncio.wait_for(asyncio.shield(self.controller._task), 20)
        self.assertIsNone(self.controller.job)
        return self.controller.result

    async def until(self, condition):
        for _ in range(500):
            if condition():
                return
            await asyncio.sleep(0.01)
        self.fail("did not happen")

    def jobs_run(self):
        return [r["argument"] for r in self.runs() if r["argument"] in ("--update", "--rebuild")]


class Install(Case):
    async def test_only_the_game_folder_is_accepted_and_it_is_remembered(self):
        with self.assertRaises(fl.LauncherError):
            self.controller.set_install(str(self.root))
        self.controller.set_install(str(self.install))
        again = fl.Controller(self.controller.settings_file, self.root)
        self.assertEqual(again.install_dir, str(self.install))

    async def test_status_and_check_run_with_no_real_display_or_the_loaders_variables(self):
        await self.ready()
        self.assertEqual(self.controller.status["version"], "1.0.0")
        self.assertEqual(len(self.controller.update["latest"]["notes"]), 200_000)  # a line longer than asyncio's usual limit
        for run in self.runs():
            self.assertNotIn("DISPLAY", run["env"])
            self.assertEqual(run["env"]["WAYLAND_DISPLAY"], fl.NO_DISPLAY["WAYLAND_DISPLAY"])
            self.assertNotIn("LD_LIBRARY_PATH", run["env"])
            self.assertEqual(run["env"]["KEPT"], "yes")
            self.assertEqual(run["env"]["HOME"], str(self.root))

    async def test_a_launcher_without_the_commands_is_named_as_too_old(self):
        (self.install / "plan.json").write_text(json.dumps({"--status": [["die", "Failed to initialize GTK"]]}))
        self.controller.set_install(str(self.install))
        await self.controller.refresh()
        self.assertIn(fl.TOO_OLD, self.controller.error)
        self.assertIn("GTK", self.controller.error)
        self.assertIsNone(self.controller.status)
        with self.assertRaises(fl.LauncherError):
            self.controller.start("rebuild")

    async def test_a_failed_update_check_drops_the_earlier_answer(self):
        await self.ready()
        self.write_plan(check_update=[["end", {"state": "error", "message": "Could not reach GitHub."}]])
        await self.controller.refresh()
        self.assertEqual((self.controller.update, self.controller.error), (None, "Could not reach GitHub."))
        self.assertEqual(self.controller.status["version"], "1.0.0")

    async def test_another_interface_version_is_refused(self):
        self.write_plan(status=[["end", dict(STATUS, api=2)]])
        self.controller.set_install(str(self.install))
        await self.controller.refresh()
        self.assertIn("interface version 2", self.controller.error)

    async def test_a_launcher_that_dies_while_its_child_holds_the_output_is_noticed(self):
        self.write_plan(status=[["child", 8], ["die", "boom"]])
        self.controller.set_install(str(self.install))
        started = time.monotonic()
        await self.controller.refresh()
        self.assertLess(time.monotonic() - started, 6)
        self.assertIn("without an answer", self.controller.error)


class Jobs(Case):
    async def test_update_goes_on_to_rebuild_with_the_sessions_display(self):
        await self.ready(
            update=[["progress", {"phase": "download", "done": 1, "total": 4, "message": "Downloading"}],
                    ["end", {"state": "done", "rebuild": True, "message": "Installed."}]],
            rebuild=[["text", "not JSON"], ["end", {"state": "done", "message": "Ready to play."}]])
        self.controller.start("update")
        with self.assertRaises(fl.LauncherError):
            self.controller.start("rebuild")
        with self.assertRaises(fl.LauncherError):
            self.controller.set_install(str(self.install))
        self.assertEqual(await self.finish(), {"state": "done", "message": "Ready to play."})
        self.assertEqual(self.done, [self.controller.result])
        self.assertEqual(self.jobs_run(), ["--update", "--rebuild"])
        displays = {r["argument"]: r["env"].get("DISPLAY") for r in self.runs()}
        self.assertEqual((displays["--update"], displays["--rebuild"]), (None, ":1"))

    async def test_an_update_that_needs_no_rebuild_stops_there(self):
        await self.ready(update=[["end", {"state": "done", "rebuild": False, "message": "Installed."}]])
        self.controller.start("update")
        self.assertEqual((await self.finish())["message"], "Installed.")
        self.assertEqual(self.jobs_run(), ["--update"])

    async def test_an_update_to_a_launcher_without_the_commands_does_not_run_it_with_a_display(self):
        older = {"--status": [["die", "Failed to initialize GTK"]]}
        for rebuild in (True, None):
            await self.ready(update=[["plan", older], ["end", {"state": "done", "rebuild": rebuild, "message": "Installed."}]])
            self.controller.start("update")
            self.assertEqual(await self.finish(), {"state": "error", "message": f"Installed. {fl.NEW_TOO_OLD}"})
            # nothing from before the update is left to offer it again
            state = self.controller.state()
            self.assertEqual((state["status"], state["update"], state["error"]), (None, None, None))
            await self.controller.refresh()
            self.assertIn(fl.TOO_OLD, self.controller.error)
        self.assertEqual(self.jobs_run(), ["--update", "--update"])
        for run in self.runs():
            if run["argument"] == "--status":
                self.assertNotIn("DISPLAY", run["env"])

    async def test_a_failed_update_does_not_rebuild(self):
        await self.ready(update=[["end", {"state": "error", "message": "No space."}]])
        self.controller.start("update")
        self.assertEqual(await self.finish(), {"state": "error", "message": "No space."})
        self.assertEqual(self.jobs_run(), ["--update"])

    async def test_progress_is_shown_and_cancel_reaches_the_launcher(self):
        await self.ready(update=[["progress", {"phase": "download", "done": 1, "total": 4, "message": "Downloading"}],
                                 ["await-cancel", None]])
        self.controller.start("update")
        await self.until(lambda: self.controller.job["message"] == "Downloading")
        self.assertEqual(self.controller.job["fraction"], 0.25)
        self.assertEqual(self.controller.state()["job"]["elapsed"], 0)
        self.controller._started -= 125  # as if it had run for a while
        self.assertEqual(self.controller.state()["job"]["elapsed"], 125)
        self.controller.cancel()
        self.assertTrue(self.controller.job["cancelling"])
        self.assertEqual((await self.finish())["state"], "cancelled")
        self.assertEqual(self.jobs_run(), ["--update"])

    async def test_a_cancel_too_late_for_the_update_still_stops_before_the_rebuild(self):
        await self.ready(update=[["progress", {"phase": "install", "done": 0, "total": 0, "message": "Installing"}], ["sleep", 0.5],
                                 ["end", {"state": "done", "rebuild": True, "message": "Installed."}]])
        self.controller.start("update")
        await self.until(lambda: self.controller.job["message"] == "Installing")
        self.controller.cancel()
        self.assertEqual(await self.finish(), {"state": "done", "message": "Installed."})
        self.assertEqual(self.jobs_run(), ["--update"])

    async def test_a_silent_shader_preparation_is_stopped_and_the_game_counts_as_ready(self):
        await self.ready(rebuild=[["progress", {"phase": "prepare", "done": 0, "total": 0, "fraction": 0.9, "message": "Preparing"}],
                                  ["await-cancel", None]])
        self.controller.prepare_timeout = 0.3
        self.controller.start("rebuild")
        result = await self.finish()
        self.assertEqual(result["state"], "done")
        self.assertIn("took too long", result["message"])

    async def test_silence_in_other_steps_is_waited_out(self):
        await self.ready(rebuild=[["progress", {"phase": "build", "done": 0, "total": 0, "message": "Compiling"}], ["sleep", 1.0],
                                  ["end", {"state": "done", "message": "Ready to play."}]])
        self.controller.prepare_timeout = 0.3
        self.controller.start("rebuild")
        self.assertEqual((await self.finish())["message"], "Ready to play.")

    async def test_unloading_cancels_the_running_job(self):
        await self.ready(rebuild=[["progress", {"phase": "build", "done": 0, "total": 0, "message": "Compiling"}],
                                  ["await-cancel", None]])
        self.controller.start("rebuild")
        await self.until(lambda: self.controller.job["message"] == "Compiling")
        pid = self.runs()[-1]["pid"]
        await asyncio.wait_for(self.controller.close(), 5)
        await self.until(lambda: not Path(f"/proc/{pid}").exists())


class Awake(Case):
    def reload(self):
        return fl.Controller(self.controller.settings_file, self.root)

    async def test_the_choice_is_on_until_turned_off_and_is_remembered(self):
        self.assertTrue(self.controller.state()["keep_awake"])
        self.controller.set_keep_awake(False)
        self.assertFalse(self.reload().keep_awake)

    async def test_the_users_timers_are_kept_until_released_even_across_a_reload(self):
        self.controller.set_install(str(self.install))
        self.controller.hold_sleep(300, 900)
        self.controller.hold_sleep(0, 0)  # a later one does not replace what was there first
        again = self.reload()
        self.assertEqual((again.held_sleep, again.install_dir), ({"battery": 300, "ac": 900}, str(self.install)))
        self.assertEqual(again.state()["held_sleep"], again.held_sleep)
        again.release_sleep()
        self.assertIsNone(self.reload().held_sleep)

    async def test_only_timers_are_recorded(self):
        for bad in ((-1, 0), (None, 3), (1.5, 2), (True, 2)):
            with self.assertRaises(fl.LauncherError):
                self.controller.hold_sleep(*bad)
        self.assertIsNone(self.controller.held_sleep)


class Display(unittest.TestCase):
    def test_the_steam_clients_display_is_taken(self):
        with tempfile.TemporaryDirectory() as tmp:
            for pid, name, env in [("3", "bash", "DISPLAY=:7"), ("20", "steam", "HOME=/h"),
                                   ("100", "steam", "DISPLAY=:1\0WAYLAND_DISPLAY=\0XAUTHORITY=/x\0SECRET=1")]:
                (Path(tmp) / pid).mkdir()
                (Path(tmp) / pid / "comm").write_text(name + "\n")
                (Path(tmp) / pid / "environ").write_text(env + "\0")
            self.assertEqual(fl.find_display(tmp), {"DISPLAY": ":1", "XAUTHORITY": "/x"})
            self.assertEqual(fl.find_display(tmp, uid=os.getuid() + 1), {})

    def test_the_users_session_is_named_when_it_is_there(self):
        with tempfile.TemporaryDirectory() as run:
            env = fl.environment({}, "/home/p", {"DISPLAY": ":1"}, run_dir=run)
            self.assertEqual(env["DBUS_SESSION_BUS_ADDRESS"], f"unix:path={run}/bus")
            self.assertEqual((env["XDG_RUNTIME_DIR"], env["DISPLAY"]), (run, ":1"))
        self.assertNotIn("XDG_RUNTIME_DIR", fl.environment({}, "/home/p", run_dir="/nowhere"))


if __name__ == "__main__":
    unittest.main()
