// SPDX-License-Identifier: GPL-3.0-or-later
// The Quick Access Menu panel: the game's folder, its version, update and rebuild. The work is in main.py; this polls it while open.
import {
  ButtonItem,
  ConfirmModal,
  DropdownItem,
  Field,
  Navigation,
  PanelSection,
  PanelSectionRow,
  showModal,
  staticClasses,
  ToggleField,
} from "@decky/ui";
import { addEventListener, callable, definePlugin, FileSelectionType, openFilePicker, removeEventListener, toaster } from "@decky/api";
import { useEffect, useRef, useState } from "react";
import { FaDownload } from "react-icons/fa";
import { AppStore, SteamApps, startGame } from "./play";
import { shownSettings, SettingsList } from "./settings";
import { SleepKeeper, SleepStatus, SteamSettings, Timers } from "./sleep";

declare const SteamClient: { Settings?: SteamSettings; Apps?: SteamApps } | undefined;
declare const appStore: AppStore | undefined;

interface Result {
  state: "done" | "error" | "cancelled";
  message: string;
}

interface State {
  install_dir: string | null;
  home: string;
  status: {
    version: string;
    busy: "game" | "job" | null;
    setup_complete: boolean;
    rebuild: boolean | null;
    rebuild_reason: string;
    problem: string | null;
    steam_shortcuts?: number[]; // not told by a launcher from before it
  } | null;
  update: { newer: boolean; latest: { version: string } | null } | null;
  settings: SettingsList | null;
  job: { step: "update" | "rebuild" | "download"; message: string; fraction: number | null; cancelling: boolean; elapsed: number } | null;
  result: Result | null;
  error: string | null;
  keep_awake: boolean;
  held_sleep: Timers | null;
}

const getState = callable<[], State>("get_state");
const watch = callable<[], State>("watch"); // the same, looking again at an install seen busy
const setInstall = callable<[folder: string], State>("set_install");
const refresh = callable<[], State>("refresh");
const start = callable<[kind: "update" | "rebuild"], State>("start");
const cancel = callable<[], State>("cancel");
const setSetting = callable<[setting: string, choice: string], State>("set_setting");
const download = callable<[kind: string, setting: string, choice: string], State>("download");
const setKeepAwake = callable<[on: boolean], State>("set_keep_awake");
const holdSleep = callable<[battery: number, ac: number], void>("hold_sleep");
const releaseSleep = callable<[], void>("release_sleep");
const note = callable<[text: string], void>("note");

const SLEEP_TEXT: Record<SleepStatus, string> = {
  off: "Auto-sleep will be turned off until this has finished. The power button still works",
  held: "Auto-sleep is off until this has finished. The power button still works",
  already: "Auto-sleep is already off in Steam's settings",
  waiting: "Reading Steam's sleep settings",
  unavailable: "Could not change Steam's auto-sleep: keep the device awake",
};

// seconds as 4:05, or 1:02:09 from an hour
const clock = (seconds: number) => {
  const two = (n: number) => String(n).padStart(2, "0");
  const h = Math.floor(seconds / 3600);
  const m = Math.floor(seconds / 60) % 60;
  return h ? `${h}:${two(m)}:${two(seconds % 60)}` : `${m}:${two(seconds % 60)}`;
};

const Note = ({ text }: { text: string }) => (
  <PanelSectionRow>
    <div style={{ fontSize: "0.85em", overflowWrap: "anywhere" }}>{text}</div>
  </PanelSectionRow>
);

// Drawn here: Decky's progress bar puts its label beside the bar, which then runs off the narrow panel.
const Progress = ({ title, message, fraction }: { title: string; message: string; fraction: number | null }) => (
  <PanelSectionRow>
    <div style={{ display: "flex", flexDirection: "column", gap: "6px", width: "100%", minWidth: 0, overflowWrap: "anywhere" }}>
      <div style={{ fontWeight: "bold" }}>{title}</div>
      <div style={{ fontSize: "0.85em" }}>{message}</div>
      {fraction !== null && (
        <div style={{ display: "flex", alignItems: "center", gap: "8px" }}>
          <div style={{ flex: 1, height: "6px", borderRadius: "3px", overflow: "hidden", background: "rgba(255, 255, 255, 0.2)" }}>
            <div style={{ width: `${Math.round(fraction * 100)}%`, height: "100%", background: "#1a9fff", transition: "width 0.5s" }} />
          </div>
          <div style={{ fontSize: "0.85em" }}>{Math.round(fraction * 100)}%</div>
        </div>
      )}
    </div>
  </PanelSectionRow>
);

const Button = (props: { label: string; onClick: () => void; disabled?: boolean }) => (
  <PanelSectionRow>
    <ButtonItem layout="below" onClick={props.onClick} disabled={props.disabled}>
      {props.label}
    </ButtonItem>
  </PanelSectionRow>
);

function Content({ sleep }: { sleep: () => SleepStatus }) {
  const [state, setState] = useState<State | null>(null);
  const [checking, setChecking] = useState(false);
  const [playFailed, setPlayFailed] = useState(false);
  const [saving, setSaving] = useState(false);
  const alive = useRef(true);
  const show = (s: State) => {
    if (alive.current) setState(s);
  };

  const check = async () => {
    setChecking(true);
    try {
      show(await refresh());
    } finally {
      if (alive.current) setChecking(false);
    }
  };

  useEffect(() => {
    alive.current = true;
    getState().then((s) => {
      show(s);
      if (s.install_dir && !s.job) check();
    });
    const timer = setInterval(() => watch().then(show), 1000);
    return () => {
      alive.current = false;
      clearInterval(timer);
    };
  }, []);

  if (!state) return <PanelSection><Note text="Loading" /></PanelSection>;
  const { status, update, job, result, error } = state;

  const choose = async () => {
    let picked;
    try {
      picked = await openFilePicker(FileSelectionType.FOLDER, state.install_dir ?? state.home, false, true);
    } catch {
      return; // the picker was closed
    }
    const next = await setInstall(picked.realpath);
    show(next);
    if (!next.error) check();
  };

  const busy = status?.busy === "game" ? "Close the game first." : status?.busy === "job" ? "The launcher is already working on this install." : null;
  const needsRebuild = status && !status.problem && (status.rebuild === true || !status.setup_complete);

  // only a game that is built for the installed release, with nothing at work on it
  const playable = status && status.setup_complete && !needsRebuild && !job && !status.busy;
  const play = () => {
    const started = startGame(
      typeof SteamClient === "undefined" ? undefined : SteamClient.Apps,
      typeof appStore === "undefined" ? undefined : appStore,
      status?.steam_shortcuts ?? [],
    );
    setPlayFailed(!started);
    if (started) Navigation.CloseSideMenus();
  };

  const pick = async (setting: string, choice: string, needs: string | null) => {
    if (needs) {
      // nothing of a vendor's is downloaded before the player has seen whose it is and on what terms, and accepted
      showModal(
        <ConfirmModal
          strTitle="Download the files?"
          strDescription={state.settings?.consent[needs]}
          strOKButtonText="I accept the licence, download"
          onOK={() => void download(needs, setting, choice).then(show)}
        />,
      );
      return;
    }
    setSaving(true);
    try {
      show(await setSetting(setting, choice));
    } finally {
      if (alive.current) setSaving(false);
    }
  };

  return (
    <>
      <PanelSection title="Game">
        {playable && !!status.steam_shortcuts?.length && <Button label="Play" onClick={play} disabled={checking} />}
        {playable && status.steam_shortcuts?.length === 0 && (
          <Note text="To play from here, add the game to Steam: in Desktop Mode, open the launcher and press Add to Steam." />
        )}
        {playFailed && <Note text="Steam did not start the game: start it from your library." />}
        <PanelSectionRow>
          <Field label="Installed version" description={state.install_dir ?? "No folder chosen"}>
            {status?.version ?? ""}
          </Field>
        </PanelSectionRow>
        <Button label="Choose the game's folder" onClick={choose} disabled={!!job} />
      </PanelSection>
      {!job && state.settings && (
        <PanelSection title="Settings">
          {shownSettings(state.settings).map((setting) => (
            <PanelSectionRow key={setting.id}>
              <DropdownItem
                label={setting.label}
                layout="below"
                strDefaultLabel="Custom"
                rgOptions={setting.options.map((o) => ({ data: o.choice, label: o.label }))}
                selectedOption={setting.current}
                disabled={saving || checking || status?.busy === "job"}
                onChange={(option) => {
                  const chosen = setting.options.find((o) => o.choice === option.data);
                  // picking the one in force again still offers its files when they are missing
                  if (chosen && (chosen.choice !== setting.current || chosen.download)) void pick(setting.id, chosen.choice, chosen.download);
                }}
              />
            </PanelSectionRow>
          ))}
          <Note text="Changes apply the next time the game starts." />
        </PanelSection>
      )}
      <PanelSection title="Update">
        {job ? (
          <>
            <Progress
              title={job.step === "update" ? "Updating" : job.step === "download" ? "Downloading" : "Building the game"}
              message={job.cancelling ? "Cancelling" : job.message}
              fraction={job.fraction}
            />
            <Note text={`Time elapsed: ${clock(job.elapsed)}`} />
            <PanelSectionRow>
              <ToggleField
                label="Prevent auto-sleep"
                description={state.keep_awake ? SLEEP_TEXT[sleep()] : "Steam's auto-sleep settings apply: the device may sleep while this runs"}
                checked={state.keep_awake}
                onChange={(on: boolean) => setKeepAwake(on).then(show)}
              />
            </PanelSectionRow>
            <Note
              text={`${job.step === "rebuild" ? "Some steps take several minutes without moving. " : ""}You can close this panel. Do not start the game until this has finished.`}
            />
            <Button label="Cancel" onClick={() => cancel().then(show)} disabled={job.cancelling} />
          </>
        ) : (
          <>
            {result && <Note text={result.message} />}
            {error && <Note text={error} />}
            {status?.problem && <Note text={`${status.problem} Open the launcher in Desktop Mode.`} />}
            {busy && <Note text={busy} />}
            {update && !update.newer && !needsRebuild && <Note text="You have the latest version." />}
            {update?.newer && (
              <Button label={`Update to ${update.latest?.version}`} onClick={() => start("update").then(show)} disabled={!!busy || checking} />
            )}
            {needsRebuild && (
              <>
                <Note text={`The game needs building${status.rebuild_reason && status.rebuild ? `: ${status.rebuild_reason}` : "."}`} />
                <Button label="Build the game" onClick={() => start("rebuild").then(show)} disabled={!!busy || checking} />
              </>
            )}
            <Button label={checking ? "Checking" : "Check for updates"} onClick={check} disabled={checking || !state.install_dir} />
          </>
        )}
      </PanelSection>
    </>
  );
}

export default definePlugin(() => {
  // also while the panel is closed
  const done = addEventListener<[result: Result]>("fefw_job_done", (result) => {
    const title = result.state === "done" ? "FEFW Recomp: finished" : result.state === "cancelled" ? "FEFW Recomp: cancelled" : "FEFW Recomp: failed";
    toaster.toast({ title, body: result.message });
  });
  // also while the panel is closed: auto-sleep follows the job, and timers left off by an earlier run are put back
  const keeper = new SleepKeeper(
    typeof SteamClient === "undefined" ? undefined : SteamClient.Settings,
    { hold: holdSleep, release: releaseSleep },
    (text) => void note(text),
  );
  const follow = async () => {
    const state = await getState();
    await keeper.tick(!!state.job && state.keep_awake, state.held_sleep);
  };
  const timer = setInterval(() => void follow(), 2000);
  return {
    name: "FEFW Recomp",
    titleView: <div className={staticClasses.Title}>FEFW Recomp</div>,
    content: <Content sleep={() => keeper.status} />,
    icon: <FaDownload />,
    onDismount() {
      removeEventListener("fefw_job_done", done);
      clearInterval(timer);
      // put the timers back now; if this is cut short the record stays, and the next load does it
      void getState()
        .then((state) => keeper.tick(false, state.held_sleep))
        .finally(() => keeper.close());
    },
  };
});
