// SPDX-License-Identifier: GPL-3.0-or-later
// Decides whether Steam's Play button on the fefw-recomp shortcut page stays as Play, switches to Set up, or shows
// a Cancel button + native-style progress readout beside it while a job runs. This file imports nothing, so the tests run it as it is.

export interface PlayButtonInput {
  status: {
    setup_complete: boolean;
    rebuild: boolean | null;
    problem: string | null;
    busy: "game" | "job" | null;
  } | null;
  job: {
    step: "update" | "rebuild" | "download";
    message?: string;
    fraction: number | null;
    cancelling: boolean;
  } | null;
  checking: boolean;
}

export type PlayButtonState =
  | { kind: "play" }
  | { kind: "setup"; label: string; disabled: boolean }
  | {
      kind: "job";
      buttonLabel: string;
      cancelling: boolean;
      statusTitle: string;
      statusDetail: string;
      fraction: number | null;
    };

// The game has to be built first: not set up yet, or built for another release. An install the launcher cannot read is not.
export const needsRebuild = (s: PlayButtonInput["status"] | undefined): boolean =>
  !!(s && !s.problem && (s.rebuild === true || !s.setup_complete));

export function resolvePlayButton(input: PlayButtonInput | null): PlayButtonState {
  if (!input) return { kind: "play" };
  if (input.job) {
    const { step, fraction, cancelling, message } = input.job;
    const pct = fraction !== null ? Math.max(0, Math.min(100, Math.round(fraction * 100))) : null;
    const statusTitle = cancelling
      ? "Cancelling"
      : step === "update"
        ? "Updating"
        : step === "download"
          ? "Downloading"
          : "Building";
    const statusDetail = pct !== null ? `${pct}% Complete` : message || "In progress";
    return {
      kind: "job",
      buttonLabel: cancelling ? "Cancelling…" : "Cancel",
      cancelling,
      statusTitle,
      statusDetail,
      fraction,
    };
  }
  const s = input.status;
  if (s && needsRebuild(s)) {
    return { kind: "setup", label: "Set up", disabled: !!s.busy || input.checking };
  }
  return { kind: "play" };
}

// Stable primitive key for useSyncExternalStore so the game page only re-renders when visible Play-bar state changes.
export function playButtonKey(state: PlayButtonState): string {
  if (state.kind === "play") return "play";
  if (state.kind === "setup") return `setup:${state.disabled ? "1" : "0"}:${state.label}`;
  return `job:${state.buttonLabel}:${state.statusTitle}:${state.statusDetail}:${state.fraction ?? ""}`;
}
