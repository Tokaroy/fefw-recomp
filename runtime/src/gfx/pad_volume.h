// SPDX-License-Identifier: GPL-3.0-or-later
// A wired controller's vibration and speaker are an audio output of its own, so their strength follows that output's volume.
// Windows: raise it to full while the haptics drive it, and put it back. Wine reports every output as full and refuses to set
// one, so nothing happens there: on Linux the launcher does this (launcher/src-tauri/src/pad_volume.rs).
// Safe to call from any thread.
#pragma once

namespace nxrt::gfx::pad_volume {

// Sets the playback endpoint of this friendly name (SDL's name for the audio device) to full volume and unmutes it, keeping
// what it was. True when something was changed.
bool raise(const char* endpoint_name);

// Puts back what raise() changed. An endpoint whose volume was changed since is left as it is; one that is gone is kept for the
// next call.
void restore();

// Whether a volume is waiting to be put back.
bool raised();

}  // namespace nxrt::gfx::pad_volume
