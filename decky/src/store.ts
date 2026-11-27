// SPDX-License-Identifier: GPL-3.0-or-later
// Shared backend calls and reactive state for both the Quick Access panel and the Steam game-page tabs.
import { callable, FileSelectionType, openFilePicker } from "@decky/api";
import { useSyncExternalStore } from "react";
import { playButtonKey, resolvePlayButton, type PlayButtonState } from "./play-button";
import { askToUpdate, type DeckyRoutes, type PluginOffer } from "./self_update";
import type { SettingsList } from "./settings";
import { isShortcutAppId } from "./shortcuts";
import type { SleepStatus, Timers } from "./sleep";

export const NOT_ASKED = "Decky could not be asked: to update the plugin, run the installer in Desktop Mode.";

export const decky = () => (window as unknown as { DeckyBackend?: DeckyRoutes }).DeckyBackend;

export interface Result {
  state: "done" | "error" | "cancelled";
  message: string;
  plugin_update?: PluginOffer; // after an update of the game that brought a newer plugin
}

export interface UpdateRelease {
  version: string;
  notes?: string;
  page?: string;
  size?: number;
}

export interface State {
  install_dir: string | null;
  home: string;
  status: {
    version: string;
    dump_dir?: string;
    busy: "game" | "job" | null;
    setup_complete: boolean;
    rebuild: boolean | null;
    rebuild_reason: string;
    problem: string | null;
    steam_shortcuts?: number[]; // not told by a launcher from before it
  } | null;
  update: { newer: boolean; latest: UpdateRelease | null } | null;
  settings: SettingsList | null;
  job: { step: "update" | "rebuild" | "download"; message: string; fraction: number | null; cancelling: boolean; elapsed: number } | null;
  result: Result | null;
  error: string | null;
  keep_awake: boolean;
  held_sleep: Timers | null;
  plugin_update: PluginOffer | null; // the installed release's plugin, when it is newer than this one
}

export const getState = callable<[], State>("get_state");
export const watch = callable<[], State>("watch");
export const setInstall = callable<[folder: string], State>("set_install");
export const refresh = callable<[], State>("refresh");
export const start = callable<[kind: "update" | "rebuild"], State>("start");
export const cancel = callable<[], State>("cancel");
export const setSetting = callable<[setting: string, choice: string], State>("set_setting");
export const download = callable<[kind: string, setting: string, choice: string], State>("download");
export const setKeepAwake = callable<[on: boolean], State>("set_keep_awake");
export const holdSleep = callable<[battery: number, ac: number], void>("hold_sleep");
export const releaseSleep = callable<[], void>("release_sleep");
export const note = callable<[text: string], void>("note");

export const SLEEP_TEXT: Record<SleepStatus, string> = {
  off: "Auto-sleep will be turned off until this has finished. The power button still works",
  held: "Auto-sleep is off until this has finished. The power button still works",
  already: "Auto-sleep is already off in Steam's settings",
  waiting: "Reading Steam's sleep settings",
  unavailable: "Could not change Steam's auto-sleep: keep the device awake",
};

// seconds as 4:05, or 1:02:09 from an hour
export const clock = (seconds: number) => {
  const two = (n: number) => String(n).padStart(2, "0");
  const h = Math.floor(seconds / 3600);
  const m = Math.floor(seconds / 60) % 60;
  return h ? `${h}:${two(m)}:${two(seconds % 60)}` : `${m}:${two(seconds % 60)}`;
};

export interface StoreSnapshot {
  state: State | null;
  checking: boolean;
  saving: boolean;
  pluginFailed: boolean;
}

let snapshot: StoreSnapshot = {
  state: null,
  checking: false,
  saving: false,
  pluginFailed: false,
};

const listeners = new Set<() => void>();
let pollTimer: ReturnType<typeof setInterval> | null = null;
let activeViews = 0;

function emit() {
  for (const listener of listeners) listener();
}

function updateSnapshot(patch: Partial<StoreSnapshot>) {
  snapshot = { ...snapshot, ...patch };
  emit();
}

export function setStateSnapshot(next: State) {
  updateSnapshot({ state: next });
}

export function setPluginFailed(failed: boolean) {
  updateSnapshot({ pluginFailed: failed });
}

export async function runCheck() {
  updateSnapshot({ checking: true });
  try {
    updateSnapshot({ state: await refresh() });
  } finally {
    updateSnapshot({ checking: false });
  }
}

export async function chooseInstallFolder() {
  const current = snapshot.state;
  if (!current) return;
  let picked;
  try {
    picked = await openFilePicker(FileSelectionType.FOLDER, current.install_dir ?? current.home, false, true);
  } catch {
    return; // the picker was closed
  }
  const next = await setInstall(picked.realpath);
  updateSnapshot({ state: next });
  if (!next.error) void runCheck();
}

export async function applySetting(setting: string, choice: string) {
  updateSnapshot({ saving: true });
  try {
    updateSnapshot({ state: await setSetting(setting, choice) });
  } finally {
    updateSnapshot({ saving: false });
  }
}

export async function startDownload(kind: string, setting: string, choice: string) {
  updateSnapshot({ state: await download(kind, setting, choice) });
}

export async function startJob(kind: "update" | "rebuild") {
  updateSnapshot({ state: await start(kind) });
}

export async function cancelJob() {
  updateSnapshot({ state: await cancel() });
}

export async function toggleKeepAwake(on: boolean) {
  updateSnapshot({ state: await setKeepAwake(on) });
}

export async function requestPluginUpdate(offer: PluginOffer) {
  const asked = await askToUpdate(decky(), offer);
  updateSnapshot({ pluginFailed: !asked });
}

// Called once at plugin init so the game-page tab patch knows the shortcut appids even before the QAM is opened.
export async function bootstrapStore() {
  try {
    const s = await getState();
    updateSnapshot({ state: s });
    if (s.install_dir && !s.status && !s.job) {
      await runCheck();
    }
  } catch {
    // will retry when a view mounts
  }
}

export function stopStore() {
  if (pollTimer !== null) {
    clearInterval(pollTimer);
    pollTimer = null;
  }
}

function subscribe(listener: () => void) {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

function getSnapshot(): StoreSnapshot {
  return snapshot;
}

// Keeps the store polling `watch()` while at least one UI surface (QAM panel or game-page tab) is mounted.
export function retainPolling(refreshIfIdle = false): () => void {
  activeViews += 1;
  if (activeViews === 1) {
    void getState().then((s) => {
      updateSnapshot({ state: s });
      if (refreshIfIdle && s.install_dir && !s.job) {
        void runCheck();
      }
    });
    pollTimer = setInterval(() => {
      void watch().then((s) => updateSnapshot({ state: s }));
    }, 1000);
  } else if (refreshIfIdle && snapshot.state?.install_dir && !snapshot.state.job && !snapshot.checking) {
    void runCheck();
  }
  return () => {
    activeViews = Math.max(0, activeViews - 1);
    if (activeViews === 0 && pollTimer !== null) {
      clearInterval(pollTimer);
      pollTimer = null;
    }
  };
}

export function useStore(): StoreSnapshot {
  return useSyncExternalStore(subscribe, getSnapshot, getSnapshot);
}

export function useIsFefwShortcut(appid: number): boolean {
  return useSyncExternalStore(
    subscribe,
    () => isShortcutAppId(appid, snapshot.state?.status?.steam_shortcuts),
    () => false,
  );
}

export function getPlayButtonState(): PlayButtonState {
  if (!snapshot.state) return { kind: "play" };
  return resolvePlayButton({
    status: snapshot.state.status,
    job: snapshot.state.job,
    checking: snapshot.checking,
  });
}

export function usePlayButtonKey(): string {
  return useSyncExternalStore(
    subscribe,
    () => playButtonKey(getPlayButtonState()),
    () => "play",
  );
}

export function hasUpdateAvailable(): boolean {
  return !!(snapshot.state?.update?.newer || snapshot.state?.plugin_update);
}

export function useHasUpdateAvailable(): boolean {
  return useSyncExternalStore(subscribe, hasUpdateAvailable, () => false);
}
