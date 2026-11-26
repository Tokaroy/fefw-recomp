// SPDX-License-Identifier: GPL-3.0-or-later
// The panel's choice of settings: node --test tests/*.test.ts
import { test } from "node:test";
import assert from "node:assert/strict";
import { shownSettings, type SettingsList } from "../src/settings.ts";

const choice = (label: string, available = true, download: string | null = null) => ({ label, available, download });
const list: SettingsList = {
  consent: {},
  settings: [
    { id: "upscaler", label: "Upscaler", current: "NVIDIA DLSS (needs Streamline files)", choices: [
      choice("Built-in temporal (TAAU)"),
      choice("NVIDIA DLSS (needs Streamline files)", false),
      choice("AMD FSR 3 (needs AMD's files)", true, "fsr"),
      choice("Intel XeSS (needs libxess.dll)", false),
    ] },
    { id: "reflections", label: "Reflections", current: null, choices: [choice("Full")] },
    { id: "preset", label: "Graphics preset", current: null, choices: [choice("Low")] },
  ],
};

test("the main settings, in the panel's order", () => {
  assert.deepEqual(shownSettings(list).map((s) => s.id), ["preset", "upscaler"]);
});

test("choices say when files come first, and unavailable ones stay only while picked", () => {
  const upscaler = shownSettings(list)[1];
  assert.deepEqual(upscaler.options.map((o) => o.label), ["Built-in temporal (TAAU)", "NVIDIA DLSS", "AMD FSR 3 (download)"]);
  assert.deepEqual(upscaler.options[2], { choice: "AMD FSR 3 (needs AMD's files)", label: "AMD FSR 3 (download)", download: "fsr" });
});
