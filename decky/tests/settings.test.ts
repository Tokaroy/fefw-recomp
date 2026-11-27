// SPDX-License-Identifier: GPL-3.0-or-later
// The panel's choice of settings: node --test tests/*.test.ts
import { test } from "node:test";
import assert from "node:assert/strict";
import { allSettings, groupedSettings, shownSettings, type SettingsList } from "../src/settings.ts";

const choice = (label: string, available = true, download: string | null = null) => ({ label, available, download });
const list: SettingsList = {
  consent: {},
  settings: [
    { id: "upscaler", label: "Upscaler", help: "Scales the image up.", current: "NVIDIA DLSS (needs Streamline files)", choices: [
      choice("Built-in temporal (TAAU)"),
      choice("NVIDIA DLSS (needs Streamline files)", false),
      choice("AMD FSR 3 (needs AMD's files)", true, "fsr"),
      choice("Intel XeSS (needs libxess.dll)", false),
    ] },
    { id: "reflections", label: "Reflections", help: "Water and floor reflections.", current: null, choices: [choice("Full")] },
    { id: "preset", label: "Graphics preset", current: null, choices: [choice("Low")] },
    { id: "npc_distance", label: "Hub NPC draw distance", current: null, choices: [] },
  ],
};

test("the main settings, in the panel's order", () => {
  assert.deepEqual(shownSettings(list).map((s) => s.id), ["preset", "upscaler"]);
});

test("all choice-based settings keep manifest order and help text", () => {
  const all = allSettings(list);
  assert.deepEqual(all.map((s) => s.id), ["upscaler", "reflections", "preset"]);
  assert.equal(all[0].help, "Scales the image up.");
  assert.equal(all[2].help, "");
});

test("choices say when files come first, and unavailable ones stay only while picked", () => {
  const upscaler = shownSettings(list)[1];
  assert.deepEqual(upscaler.options.map((o) => o.label), ["Built-in temporal (TAAU)", "NVIDIA DLSS", "AMD FSR 3 (download)"]);
  assert.deepEqual(upscaler.options[2], { choice: "AMD FSR 3 (needs AMD's files)", label: "AMD FSR 3 (download)", download: "fsr" });
});

test("groupedSettings groups settings like the launcher and puts unknown ones under Other", () => {
  const withExtra: SettingsList = {
    consent: {},
    settings: [
      ...list.settings,
      { id: "future_opt", label: "Future option", current: null, choices: [choice("On")] },
    ],
  };
  const groups = groupedSettings(withExtra);
  assert.deepEqual(
    groups.map((g) => [g.title, g.settings.map((s) => s.id)]),
    [
      ["Preset", ["preset"]],
      ["Upscaling and frame generation", ["upscaler"]],
      ["Detail", ["reflections"]],
      ["Other", ["future_opt"]],
    ],
  );
});
