// SPDX-License-Identifier: GPL-3.0-or-later
// How the launcher's settings (--settings) read in a drop-down, both for the narrow Quick Access panel (the four changed most) and
// the full-width game page tab (all of them). This file imports nothing, so the tests run it as it is.

export interface Choice {
  label: string;
  available: boolean;
  download: string | null; // files to fetch first
}

export interface SettingItem {
  id: string;
  label: string;
  help?: string;
  current: string | null;
  choices: Choice[];
}

export interface SettingsList {
  settings: SettingItem[];
  consent: Record<string, string>;
}

export interface FormattedOption {
  choice: string;
  label: string;
  download: string | null;
}

export interface FormattedSetting {
  id: string;
  label: string;
  help: string;
  current: string | null;
  options: FormattedOption[];
}

export interface SettingGroup {
  title: string;
  settings: FormattedSetting[];
}

export const SHOWN = ["preset", "fps_target", "output", "upscaler"];

// Matches the launcher's Settings page section headings (launcher/src/pages/settings.js & locales/en.js).
// Any setting returned by a newer launcher that isn't listed here is appended under "Other".
export const GROUPS: ReadonlyArray<{ title: string; ids: readonly string[] }> = [
  { title: "Preset", ids: ["preset"] },
  { title: "Resolution", ids: ["output", "ultrawide"] },
  { title: "Frame rate", ids: ["fps_target", "smooth_step"] },
  { title: "Upscaling and frame generation", ids: ["upscaler", "render_ratio", "frame_gen"] },
  {
    title: "Detail",
    ids: ["quality", "shadow_size", "reflections", "ambient_occlusion", "crowd_detail", "scenery_detail", "texture_detail", "astc_bc"],
  },
  { title: "Mouse", ids: ["jc2_mouse"] },
];

// The manifest says what an upscaler needs ("(needs AMD's files)"); here the choice itself says whether they are still to come.
const plain = (label: string) => label.replace(/\s*\(needs [^)]*\)$/, "");

const formatSetting = (s: SettingItem): FormattedSetting => ({
  id: s.id,
  label: s.label,
  help: s.help ?? "",
  current: s.current,
  // what cannot be picked here stays listed only while it is what is picked
  options: s.choices
    .filter((c) => c.available || c.label === s.current)
    .map((c) => ({ choice: c.label, label: c.download ? `${plain(c.label)} (download)` : plain(c.label), download: c.download })),
});

export const shownSettings = (list: SettingsList): FormattedSetting[] =>
  SHOWN.flatMap((id) => list.settings.filter((s) => s.id === id)).map(formatSetting);

export const allSettings = (list: SettingsList): FormattedSetting[] =>
  list.settings.filter((s) => s.choices.length > 0).map(formatSetting);

export const groupedSettings = (list: SettingsList): SettingGroup[] => {
  const formatted = allSettings(list);
  const byId = new Map(formatted.map((s) => [s.id, s]));
  const placed = new Set<string>();
  const groups: SettingGroup[] = [];

  for (const def of GROUPS) {
    const items: FormattedSetting[] = [];
    for (const id of def.ids) {
      const s = byId.get(id);
      if (s) {
        items.push(s);
        placed.add(id);
      }
    }
    if (items.length > 0) groups.push({ title: def.title, settings: items });
  }

  const remaining = formatted.filter((s) => !placed.has(s.id));
  if (remaining.length > 0) groups.push({ title: "Other", settings: remaining });
  return groups;
};
