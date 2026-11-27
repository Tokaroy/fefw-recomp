// SPDX-License-Identifier: GPL-3.0-or-later
// Play button state logic: node --test tests/*.test.ts
import { test } from "node:test";
import assert from "node:assert/strict";
import { playButtonKey, resolvePlayButton } from "../src/play-button.ts";

const ready = { setup_complete: true, rebuild: false, problem: null, busy: null };

test("ready game keeps Steam's Play button", () => {
  assert.deepEqual(resolvePlayButton(null), { kind: "play" });
  assert.deepEqual(resolvePlayButton({ status: ready, job: null, checking: false }), { kind: "play" });
  assert.deepEqual(
    resolvePlayButton({ status: { ...ready, setup_complete: false, problem: "No dump" }, job: null, checking: false }),
    { kind: "play" },
  );
});

test("needs rebuild or unfinished setup switches Play to Set up", () => {
  assert.deepEqual(
    resolvePlayButton({ status: { ...ready, rebuild: true }, job: null, checking: false }),
    { kind: "setup", label: "Set up", disabled: false },
  );
  assert.deepEqual(
    resolvePlayButton({ status: { ...ready, setup_complete: false }, job: null, checking: true }),
    { kind: "setup", label: "Set up", disabled: true },
  );
  assert.deepEqual(
    resolvePlayButton({ status: { ...ready, rebuild: true, busy: "job" }, job: null, checking: false }),
    { kind: "setup", label: "Set up", disabled: true },
  );
});

test("running job shows Cancel button and native-style progress readout", () => {
  assert.deepEqual(
    resolvePlayButton({ status: ready, job: { step: "rebuild", fraction: 0.424, cancelling: false }, checking: false }),
    {
      kind: "job",
      buttonLabel: "Cancel",
      cancelling: false,
      statusTitle: "Building",
      statusDetail: "42% Complete",
      fraction: 0.424,
    },
  );
  assert.deepEqual(
    resolvePlayButton({ status: ready, job: { step: "update", message: "Downloading release", fraction: null, cancelling: false }, checking: false }),
    {
      kind: "job",
      buttonLabel: "Cancel",
      cancelling: false,
      statusTitle: "Updating",
      statusDetail: "Downloading release",
      fraction: null,
    },
  );
  assert.deepEqual(
    resolvePlayButton({ status: ready, job: { step: "download", fraction: 0.5, cancelling: false }, checking: false }),
    {
      kind: "job",
      buttonLabel: "Cancel",
      cancelling: false,
      statusTitle: "Downloading",
      statusDetail: "50% Complete",
      fraction: 0.5,
    },
  );
  assert.deepEqual(
    resolvePlayButton({ status: ready, job: { step: "rebuild", fraction: 0.5, cancelling: true }, checking: false }),
    {
      kind: "job",
      buttonLabel: "Cancelling…",
      cancelling: true,
      statusTitle: "Cancelling",
      statusDetail: "50% Complete",
      fraction: 0.5,
    },
  );
});

test("playButtonKey changes only when visible state changes", () => {
  assert.equal(playButtonKey({ kind: "play" }), "play");
  assert.notEqual(
    playButtonKey({ kind: "setup", label: "Set up", disabled: false }),
    playButtonKey({ kind: "setup", label: "Set up", disabled: true }),
  );
});
