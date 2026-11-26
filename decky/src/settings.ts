// SPDX-License-Identifier: GPL-3.0-or-later
// Which of the launcher's settings the panel shows (--settings lists them all; it is narrow, so the ones changed most), and how
// their choices read in a drop-down. This file imports nothing, so the tests run it as it is.

export interface Choice {
  label: string;
  available: boolean;
  download: string | null; // files to fetch first
}

export interface SettingsList {
  settings: { id: string; label: string; current: string | null; choices: Choice[] }[];
  consent: Record<string, string>;
}

export const SHOWN = ["preset", "fps_target", "output", "upscaler"];

// The manifest says what an upscaler needs ("(needs AMD's files)"); here the choice itself says whether they are still to come.
const plain = (label: string) => label.replace(/\s*\(needs [^)]*\)$/, "");

export const shownSettings = (list: SettingsList) =>
  SHOWN.flatMap((id) => list.settings.filter((s) => s.id === id)).map((s) => ({
    id: s.id,
    label: s.label,
    current: s.current,
    // what cannot be picked here stays listed only while it is what is picked
    options: s.choices
      .filter((c) => c.available || c.label === s.current)
      .map((c) => ({ choice: c.label, label: c.download ? `${plain(c.label)} (download)` : plain(c.label), download: c.download })),
  }));
