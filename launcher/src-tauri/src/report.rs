// SPDX-License-Identifier: GPL-3.0-or-later
//! Opt-in, anonymized crash and performance report uploader.
//!
//! Runs after `fefw.exe` exits when the player has opted in (`Config::send_reports`). Re-reads the saved preference on exit so
//! opting out while the game runs stops that run's upload. Only fires when an issue is detected (fresh crash/freeze/fatal file,
//! non-zero exit code, heavy hitches, or sustained low FPS), scrubs local paths (including configured `fefw.ini` paths such as
//! `fs.save_dir`), usernames, hostnames and Steam IDs, saves the exact scrubbed JSON to `fefw_last_report.json` in the run
//! folder so the player can inspect it, compresses it with zstd, and POSTs it to the configured Cloudflare Worker.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::ini::Ini;
use crate::play::ReportFile;
use crate::settings::GAME_INI;

/// Baked-in report endpoint URL (set `FEFW_REPORT_URL` at build time, or override at runtime via env).
/// Empty disables uploads until a Worker URL is configured.
pub const DEFAULT_URL: &str = match option_env!("FEFW_REPORT_URL") {
    Some(u) => u,
    None => "",
};

const MAX_LOG_HEAD: usize = 64 * 1024;
const MAX_LOG_TAIL: usize = 512 * 1024;
const MAX_CRASH_FILE: usize = 256 * 1024;
const MAX_COMPRESSED_BYTES: usize = 1_572_864; // 1.5 MiB (matches Worker limit)
const DAY_SECS: u64 = 24 * 60 * 60;

/// Options passed from `main.rs` into `play::start` for post-exit reporting.
#[derive(Clone, Default)]
pub struct Options {
    pub config_dir: Option<PathBuf>,
    pub dump_dir: String,
    pub launcher_dir: PathBuf,
    pub launcher_version: String,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct SavedPref {
    send_reports: bool,
}

#[derive(Debug, Default, PartialEq)]
struct LogMetrics {
    measured_s: f64,
    mean_fps: Option<f64>,
    low1_fps: Option<f64>,
    worst_ms: Option<f64>,
    hitch_count: u32,
    build_id: String,
    gpu: String,
}

#[derive(Serialize)]
struct HeaderMeta<'a> {
    trigger: &'a str,
    build_id: &'a str,
    launcher_version: &'a str,
    os: &'a str,
    gpu: &'a str,
    exit_code: Option<i32>,
    mean_fps: Option<f64>,
    low1_fps: Option<f64>,
    worst_ms: Option<f64>,
    hitch_count: u32,
    crash_cause: Option<&'a str>,
}

#[derive(Serialize)]
struct ReportBundle<'a> {
    schema: u32,
    trigger: &'a str,
    build_id: &'a str,
    launcher_version: &'a str,
    os: &'a str,
    arch: &'a str,
    gpu: &'a str,
    exit_code: Option<i32>,
    mean_fps: Option<f64>,
    low1_fps: Option<f64>,
    worst_ms: Option<f64>,
    hitch_count: u32,
    crash_cause: Option<&'a str>,
    settings: BTreeMap<String, String>,
    crash_files: BTreeMap<String, String>,
    log: String,
}

#[derive(Serialize, Deserialize, Default)]
struct RateState {
    sent: Vec<SentEntry>,
}

#[derive(Serialize, Deserialize, Clone)]
struct SentEntry {
    ts: u64,
    trigger: String,
    build_id: String,
    sig: String,
}

fn endpoint_url() -> Option<String> {
    let raw = std::env::var("FEFW_REPORT_URL").ok().filter(|s| !s.trim().is_empty()).unwrap_or_else(|| DEFAULT_URL.to_string());
    let trimmed = raw.trim().trim_end_matches('/').to_string();
    if trimmed.starts_with("https://") || trimmed.starts_with("http://") {
        Some(trimmed)
    } else {
        None
    }
}

/// Re-reads the saved `launcher.json` so opting out while the game runs prevents that run from uploading.
fn is_enabled_on_disk(config_dir: &Path) -> bool {
    fs::read_to_string(config_dir.join("launcher.json")).ok().and_then(|t| serde_json::from_str::<SavedPref>(&t).ok()).is_some_and(|p| p.send_reports)
}

/// Checks whether the finished run had a crash or performance issue and, if opted in and within rate limits,
/// scrubs and uploads an anonymized report in the background. Errors are ignored so reporting never disrupts play.
pub fn maybe_send(opts: &Options, game_dir: &Path, code: Option<i32>, files: &[ReportFile], crash_cause: Option<&str>) {
    let Some(cfg_dir) = opts.config_dir.as_deref() else {
        return;
    };
    if !is_enabled_on_disk(cfg_dir) {
        return;
    }
    let Some(base_url) = endpoint_url() else {
        return;
    };

    let log_raw = read_log_bounded(&game_dir.join("fefw.log"));
    let metrics = parse_log_metrics(&log_raw);
    let Some(trigger) = detect_trigger(code, files, &metrics) else {
        return;
    };

    let build_id =
        if !metrics.build_id.is_empty() { metrics.build_id.clone() } else { crate::update::BUILD_ID.unwrap_or(&opts.launcher_version).to_string() };

    let now = now_unix_s();
    let sig = crash_cause.unwrap_or("").chars().take(80).collect::<String>();
    if !allow_by_rate_limit(cfg_dir, now, trigger, &build_id, &sig) {
        return;
    }

    let ini_text = fs::read_to_string(game_dir.join(GAME_INI)).unwrap_or_default();
    let ini = Ini::parse(&ini_text);

    let scrubber = Scrubber::from_env(game_dir, &opts.dump_dir, &opts.launcher_dir, &ini);
    let gpu = {
        let g = scrubber.scrub(&metrics.gpu);
        if g.trim().is_empty() {
            "unknown".to_string()
        } else {
            g
        }
    };
    let clean_cause = crash_cause.map(|c| scrubber.scrub(c));
    let clean_log = scrubber.scrub(&slice_log(&log_raw, MAX_LOG_HEAD, MAX_LOG_TAIL));

    let mut crash_files = BTreeMap::new();
    for f in files {
        if f.fresh && f.name != "fefw.log" {
            if let Ok(bytes) = fs::read(game_dir.join(&f.name)) {
                let text = String::from_utf8_lossy(&bytes[..bytes.len().min(MAX_CRASH_FILE)]);
                crash_files.insert(f.name.clone(), scrubber.scrub(&text));
            }
        }
    }

    let settings = safe_ini_settings(&ini);
    let bundle = ReportBundle {
        schema: 1,
        trigger,
        build_id: &build_id,
        launcher_version: &opts.launcher_version,
        os: std::env::consts::OS,
        arch: std::env::consts::ARCH,
        gpu: &gpu,
        exit_code: code,
        mean_fps: metrics.mean_fps,
        low1_fps: metrics.low1_fps,
        worst_ms: metrics.worst_ms,
        hitch_count: metrics.hitch_count,
        crash_cause: clean_cause.as_deref(),
        settings,
        crash_files,
        log: clean_log,
    };

    let Ok(json_bytes) = serde_json::to_vec_pretty(&bundle) else {
        return;
    };
    // Save the exact scrubbed payload locally so the player can inspect what was sent.
    let _ = fs::write(game_dir.join("fefw_last_report.json"), &json_bytes);

    let Ok(compressed) = zstd::stream::encode_all(json_bytes.as_slice(), 9) else {
        return;
    };
    if compressed.is_empty() || compressed.len() > MAX_COMPRESSED_BYTES {
        return;
    }

    let meta = HeaderMeta {
        trigger,
        build_id: &build_id,
        launcher_version: &opts.launcher_version,
        os: std::env::consts::OS,
        gpu: &gpu,
        exit_code: code,
        mean_fps: metrics.mean_fps,
        low1_fps: metrics.low1_fps,
        worst_ms: metrics.worst_ms,
        hitch_count: metrics.hitch_count,
        crash_cause: clean_cause.as_deref(),
    };
    let Ok(meta_json) = serde_json::to_vec(&meta) else {
        return;
    };
    let meta_b64 = base64_encode(&meta_json);

    let tmp = std::env::temp_dir().join(format!("fefw-report-{}-{now}.zst", std::process::id()));
    if fs::write(&tmp, &compressed).is_err() {
        return;
    }
    let url = format!("{base_url}/v1/report");
    let ok = crate::update::curl()
        .args([
            "--max-time",
            "20",
            "--request",
            "POST",
            "--header",
            "Content-Type: application/zstd",
            "--header",
            &format!("X-Report-Meta: {meta_b64}"),
            "--data-binary",
            &format!("@{}", tmp.display()),
            &url,
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    let _ = fs::remove_file(&tmp);

    if ok {
        record_sent(cfg_dir, now, trigger, &build_id, &sig);
    }
}

fn now_unix_s() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// Reads a log file; if it is very large (> 8 MiB), keeps the first 128 KiB and last 2 MiB so scanning stays fast.
fn read_log_bounded(path: &Path) -> String {
    let Ok(md) = fs::metadata(path) else {
        return String::new();
    };
    let len = md.len();
    if len <= 8 * 1024 * 1024 {
        return fs::read(path).map(|b| String::from_utf8_lossy(&b).into_owned()).unwrap_or_default();
    }
    use std::io::{Read, Seek, SeekFrom};
    let Ok(mut f) = fs::File::open(path) else {
        return String::new();
    };
    let head_len = 128 * 1024;
    let tail_len = 2 * 1024 * 1024;
    let mut head = vec![0u8; head_len];
    let n1 = f.read(&mut head).unwrap_or(0);
    head.truncate(n1);
    let mut tail = vec![0u8; tail_len];
    let _ = f.seek(SeekFrom::End(-(tail_len as i64)));
    let n2 = f.read(&mut tail).unwrap_or(0);
    tail.truncate(n2);
    format!(
        "{}\n... [skipped {} bytes] ...\n{}",
        String::from_utf8_lossy(&head),
        len.saturating_sub((n1 + n2) as u64),
        String::from_utf8_lossy(&tail)
    )
}

fn slice_log(text: &str, head_max: usize, tail_max: usize) -> String {
    if text.len() <= head_max + tail_max {
        return text.to_string();
    }
    let head_end = floor_char_boundary(text, head_max);
    let tail_start = ceil_char_boundary(text, text.len().saturating_sub(tail_max));
    let omitted = tail_start.saturating_sub(head_end);
    format!("{}\n... [trimmed {} bytes] ...\n{}", &text[..head_end], omitted, &text[tail_start..])
}

fn floor_char_boundary(s: &str, mut idx: usize) -> usize {
    idx = idx.min(s.len());
    while idx > 0 && !s.is_char_boundary(idx) {
        idx -= 1;
    }
    idx
}

fn ceil_char_boundary(s: &str, mut idx: usize) -> usize {
    idx = idx.min(s.len());
    while idx < s.len() && !s.is_char_boundary(idx) {
        idx += 1;
    }
    idx
}

fn detect_trigger(code: Option<i32>, files: &[ReportFile], m: &LogMetrics) -> Option<&'static str> {
    if files.iter().any(|f| f.fresh && f.name == "fefw_crash.txt") {
        return Some("crash");
    }
    if files.iter().any(|f| f.fresh && f.name == "fefw_freeze.txt") {
        return Some("freeze");
    }
    if files.iter().any(|f| f.fresh && f.name == "fefw_fatal.txt") {
        return Some("fatal");
    }
    if code != Some(0) {
        return Some("exit_code");
    }
    if m.worst_ms.is_some_and(|w| w >= 250.0) || (m.measured_s >= 60.0 && m.hitch_count >= 10) {
        return Some("hitch");
    }
    if m.measured_s >= 120.0 && (m.mean_fps.is_some_and(|f| f < 28.0) || m.low1_fps.is_some_and(|f| f < 18.0)) {
        return Some("low_fps");
    }
    None
}

fn parse_log_metrics(log: &str) -> LogMetrics {
    let mut m = LogMetrics::default();
    let mut weighted_fps = 0.0;
    let mut weight_s = 0.0;

    for line in log.lines() {
        // Hitch warnings from gauge.cpp: `[  12.345 W t123 hitch] ...`
        if line.contains(" W ") && line.contains("hitch") {
            m.hitch_count = m.hitch_count.saturating_add(1);
        }
        // Summary lines from gauge.cpp:
        // `3600 frames in 60.0 s: mean 60.0 fps, 1% low 52.3 fps, worst 28.4 ms, ...`
        if let Some((span_s, mean, low1, worst)) = parse_hitch_summary(line) {
            m.measured_s += span_s;
            weighted_fps += mean * span_s;
            weight_s += span_s;
            m.low1_fps = Some(m.low1_fps.map_or(low1, |prev| prev.min(low1)));
            m.worst_ms = Some(m.worst_ms.map_or(worst, |prev| prev.max(worst)));
        }
        // Runtime session stamp (runtime/src/core/build_info.cpp):
        // `runtime: session: <app_build>, ABI <N>; gpu <name>; ...`
        if let Some((_, rest)) = line.split_once("runtime: session:") {
            parse_session_stamp(rest, &mut m);
        } else if m.gpu.is_empty() {
            if let Some((_, rest)) = line.split_once("Vulkan device:") {
                let g = rest.split([',', ';', '|']).next().unwrap_or("").trim();
                if !g.is_empty() {
                    m.gpu = g.chars().take(128).collect();
                }
            }
        }
    }

    if weight_s > 0.0 {
        m.mean_fps = Some((weighted_fps / weight_s * 10.0).round() / 10.0);
    }
    m
}

fn parse_session_stamp(rest: &str, m: &mut LogMetrics) {
    let mut parts = rest.split(';');
    if let Some(first) = parts.next() {
        let head = first.split(", ABI").next().unwrap_or(first).trim();
        if m.build_id.is_empty() && !head.is_empty() && !head.starts_with("nxrt (no build") {
            m.build_id = head.chars().take(64).collect();
        }
    }
    for part in parts {
        let p = part.trim();
        if let Some(v) = p.strip_prefix("gpu ").or_else(|| p.strip_prefix("GPU ")) {
            let v = v.trim();
            if !v.is_empty() {
                m.gpu = v.chars().take(128).collect();
            }
        }
    }
}

fn parse_hitch_summary(line: &str) -> Option<(f64, f64, f64, f64)> {
    let (_, after_in) = line.split_once("frames in ")?;
    let (span_str, after_s) = after_in.split_once(" s: mean ")?;
    let (mean_str, after_mean) = after_s.split_once(" fps, 1% low ")?;
    let (low1_str, after_low1) = after_mean.split_once(" fps, worst ")?;
    let (worst_str, _) = after_low1.split_once(" ms")?;
    let span: f64 = span_str.trim().parse().ok()?;
    let mean: f64 = mean_str.trim().parse().ok()?;
    let low1: f64 = low1_str.trim().parse().ok()?;
    let worst: f64 = worst_str.trim().parse().ok()?;
    if span > 0.0 && mean.is_finite() && low1.is_finite() && worst.is_finite() {
        Some((span, mean, low1, worst))
    } else {
        None
    }
}

/// Allowlisted non-path keys from `fefw.ini` matching the actual sections/keys written by the launcher.
fn safe_ini_settings(ini: &Ini) -> BTreeMap<String, String> {
    const ALLOW: &[(&str, &str)] = &[
        ("fefw", "output"),
        ("fefw", "ultrawide"),
        ("fefw", "fps60"),
        ("fefw", "fps120"),
        ("fefw", "smooth_step"),
        ("fefw", "render"),
        ("fefw", "render_ratio"),
        ("fefw", "quality"),
        ("fefw", "shadow_size"),
        ("fefw", "reflections"),
        ("fefw", "ambient_occlusion"),
        ("fefw", "crowd_detail"),
        ("fefw", "scenery_detail"),
        ("display", "window_mode"),
        ("display", "width"),
        ("display", "height"),
        ("dlss", "upscaler"),
        ("dlss", "frame_gen"),
        ("gfx", "texture_detail"),
        ("gfx", "astc_bc"),
        ("log", "level"),
    ];
    let mut out = BTreeMap::new();
    for &(sec, key) in ALLOW {
        if let Some(val) = ini.get(sec, key) {
            let v = val.trim();
            if !v.is_empty() && v.len() <= 64 {
                out.insert(format!("{sec}.{key}"), v.to_string());
            }
        }
    }
    out
}

fn rate_state_path(config_dir: &Path) -> PathBuf {
    config_dir.join("report_state.json")
}

fn load_rate_state(config_dir: &Path, now: u64) -> RateState {
    let mut st: RateState = fs::read_to_string(rate_state_path(config_dir)).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
    st.sent.retain(|e| now.saturating_sub(e.ts) < DAY_SECS);
    st
}

fn allow_by_rate_limit(config_dir: &Path, now: u64, trigger: &str, build_id: &str, sig: &str) -> bool {
    let st = load_rate_state(config_dir, now);
    let is_perf = matches!(trigger, "hitch" | "low_fps");
    if is_perf {
        !st.sent.iter().any(|e| matches!(e.trigger.as_str(), "hitch" | "low_fps") && e.build_id == build_id)
    } else {
        let crash_count = st.sent.iter().filter(|e| !matches!(e.trigger.as_str(), "hitch" | "low_fps")).count();
        if crash_count >= 3 {
            return false;
        }
        !st.sent.iter().any(|e| e.trigger == trigger && e.build_id == build_id && e.sig == sig)
    }
}

fn record_sent(config_dir: &Path, now: u64, trigger: &str, build_id: &str, sig: &str) {
    let mut st = load_rate_state(config_dir, now);
    st.sent.push(SentEntry { ts: now, trigger: trigger.to_string(), build_id: build_id.to_string(), sig: sig.to_string() });
    if let Ok(bytes) = serde_json::to_vec_pretty(&st) {
        let _ = fs::write(rate_state_path(config_dir), bytes);
    }
}

struct Scrubber {
    /// Exact literal replacements, longest first.
    literals: Vec<(String, &'static str)>,
}

impl Scrubber {
    fn from_env(game_dir: &Path, dump_dir: &str, launcher_dir: &Path, ini: &Ini) -> Self {
        let mut s = Self { literals: Vec::new() };
        s.add_path(&game_dir.to_string_lossy(), "<GAME_DIR>");
        if !dump_dir.trim().is_empty() {
            s.add_path(dump_dir.trim(), "<DUMP_DIR>");
        }
        s.add_path(&launcher_dir.to_string_lossy(), "<LAUNCHER_DIR>");
        // Include all configured paths from fefw.ini (e.g. custom save_dir, mods, dump, caches, upscaler DLLs).
        for (sec, key, token) in [
            ("fs", "save_dir", "<SAVE_DIR>"),
            ("fs", "romfs", "<ROMFS_DIR>"),
            ("fs", "exefs", "<EXEFS_DIR>"),
            ("fs", "mods", "<MODS_DIR>"),
            ("aoc", "dir", "<AOC_DIR>"),
            ("nvn", "shader_cache", "<SHADER_CACHE>"),
            ("nvn", "bc7_cache", "<BC7_CACHE>"),
            ("upscale", "fsr_dll", "<UPSCALER_PATH>"),
            ("upscale", "dlss_dll", "<UPSCALER_PATH>"),
            ("upscale", "xess_dll", "<UPSCALER_PATH>"),
        ] {
            if let Some(val) = ini.get(sec, key) {
                s.add_path(&val, token);
            }
        }
        for var in ["HOME", "USERPROFILE"] {
            if let Ok(v) = std::env::var(var) {
                s.add_path(&v, "<HOME>");
            }
        }
        for var in ["COMPUTERNAME", "HOSTNAME"] {
            if let Ok(v) = std::env::var(var) {
                let v = v.trim();
                if v.len() >= 4 {
                    s.literals.push((v.to_string(), "<HOST>"));
                }
            }
        }
        s.literals.sort_by_key(|a| std::cmp::Reverse(a.0.len()));
        s.literals.dedup_by(|a, b| a.0 == b.0);
        s
    }

    fn add_path(&mut self, raw: &str, token: &'static str) {
        let trimmed = raw.trim().trim_matches('"').trim_end_matches(['/', '\\']);
        if trimmed.len() < 4 {
            return;
        }
        let fwd = trimmed.replace('\\', "/");
        let bwd = trimmed.replace('/', "\\");
        self.literals.push((fwd.clone(), token));
        if bwd != fwd {
            self.literals.push((bwd, token));
        }
        if let Some(stripped) = fwd.strip_prefix("Z:").or_else(|| fwd.strip_prefix("z:")) {
            if stripped.starts_with('/') && stripped.len() >= 4 {
                self.literals.push((stripped.to_string(), token));
            }
        } else if fwd.starts_with('/') {
            self.literals.push((format!("Z:{fwd}"), token));
            self.literals.push((format!("z:{fwd}"), token));
            self.literals.push((format!("Z:{}", fwd.replace('/', "\\")), token));
            self.literals.push((format!("z:{}", fwd.replace('/', "\\")), token));
        }
    }

    fn scrub(&self, input: &str) -> String {
        let mut out = input.to_string();
        for (needle, repl) in &self.literals {
            if !needle.is_empty() && out.contains(needle.as_str()) {
                out = out.replace(needle.as_str(), repl);
            }
        }
        out = scrub_user_segments(&out);
        scrub_steam64_ids(&out)
    }
}

/// Replaces usernames following common home-directory prefixes (`/home/<user>`, `/Users/<user>`, `Users\<user>`, `Users/<user>`).
fn scrub_user_segments(input: &str) -> String {
    const PREFIXES: &[&str] = &["/home/", "/Users/", "Users\\", "Users/", "users\\", "users/"];
    let mut out = input.to_string();
    for prefix in PREFIXES {
        let mut next = String::with_capacity(out.len());
        let mut rest = out.as_str();
        while let Some(pos) = rest.find(prefix) {
            let start = pos + prefix.len();
            next.push_str(&rest[..start]);
            let after = &rest[start..];
            let seg_len = after.find(['/', '\\', ' ', '\t', '\r', '\n', '"', '\'', ',', ';', ':', ')', ']']).unwrap_or(after.len());
            let seg = &after[..seg_len];
            if !seg.is_empty() && seg != "<USER>" && seg != "<HOME>" {
                next.push_str("<USER>");
            } else {
                next.push_str(seg);
            }
            rest = &after[seg_len..];
        }
        next.push_str(rest);
        out = next;
    }
    out
}

/// Redacts 17-digit Steam64 IDs (`7656119xxxxxxxxxx`).
fn scrub_steam64_ids(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = String::with_capacity(input.len());
    let mut i = 0;
    while i < bytes.len() {
        if i + 17 <= bytes.len()
            && &bytes[i..i + 7] == b"7656119"
            && bytes[i + 7..i + 17].iter().all(|b| b.is_ascii_digit())
            && (i == 0 || !bytes[i - 1].is_ascii_digit())
            && (i + 17 == bytes.len() || !bytes[i + 17].is_ascii_digit())
        {
            out.push_str("<STEAM_ID>");
            i += 17;
        } else {
            let ch = input[i..].chars().next().unwrap();
            out.push(ch);
            i += ch.len_utf8();
        }
    }
    out
}

fn base64_encode(input: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(input.len().div_ceil(3) * 4);
    for chunk in input.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = *chunk.get(1).unwrap_or(&0) as u32;
        let b2 = *chunk.get(2).unwrap_or(&0) as u32;
        let n = (b0 << 16) | (b1 << 8) | b2;
        out.push(TABLE[((n >> 18) & 63) as usize] as char);
        out.push(TABLE[((n >> 12) & 63) as usize] as char);
        out.push(if chunk.len() > 1 { TABLE[((n >> 6) & 63) as usize] as char } else { '=' });
        out.push(if chunk.len() > 2 { TABLE[(n & 63) as usize] as char } else { '=' });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scrubs_paths_save_dir_usernames_and_steam_ids() {
        let ini = Ini::parse("[fs]\nsave_dir = D:/Alice Backups/FEFW\nmods = Z:/mnt/alice_mods/fefw\n");
        let mut s = Scrubber::from_env(Path::new("/home/alice/Games/fefw"), "", Path::new("/opt/launcher"), &ini);
        s.literals.push(("ALICE-PC".into(), "<HOST>"));
        s.literals.sort_by_key(|a| std::cmp::Reverse(a.0.len()));
        let sample = "save=D:/Alice Backups/FEFW/slot0 mods=/mnt/alice_mods/fefw Loaded Z:\\home\\alice\\Games\\fefw\\fefw.ini on ALICE-PC from C:\\Users\\bob\\dump and /home/charlie/test steam=76561198012345678";
        let clean = s.scrub(sample);
        assert!(!clean.contains("Alice Backups"));
        assert!(!clean.contains("alice_mods"));
        assert!(!clean.contains("alice"));
        assert!(!clean.contains("bob"));
        assert!(!clean.contains("charlie"));
        assert!(!clean.contains("ALICE-PC"));
        assert!(!clean.contains("76561198012345678"));
        assert!(clean.contains("<SAVE_DIR>"));
        assert!(clean.contains("<MODS_DIR>"));
        assert!(clean.contains("<GAME_DIR>"));
        assert!(clean.contains("<USER>"));
        assert!(clean.contains("<HOST>"));
        assert!(clean.contains("<STEAM_ID>"));
    }

    #[test]
    fn parses_real_session_stamp_and_ignores_bc_gpu_line() {
        let log = "\
[  0.100 I t1 gfx] texture compression on this GPU: BC yes, ASTC no\n\
[  1.000 I t1 runtime] runtime: session: fefw 0.3.1 (abc1234), ABI 4; gpu AMD Radeon RX 7800 XT; wine 9.0\n\
[ 60.000 I t2 hitch] 1500 frames in 60.0 s: mean 25.0 fps, 1% low 16.5 fps, worst 80.0 ms, 4 over 2x the median (40.0 ms)\n\
[120.000 I t2 hitch] 1560 frames in 60.0 s: mean 26.0 fps, 1% low 17.0 fps, worst 90.0 ms, 2 over 2x the median (38.5 ms)\n";
        let m = parse_log_metrics(log);
        assert_eq!(m.build_id, "fefw 0.3.1 (abc1234)");
        assert_eq!(m.gpu, "AMD Radeon RX 7800 XT");
        assert_eq!(m.mean_fps, Some(25.5));
        assert_eq!(m.low1_fps, Some(16.5));
        assert_eq!(m.worst_ms, Some(90.0));
        assert_eq!(detect_trigger(Some(0), &[], &m), Some("low_fps"));

        let healthy =
            LogMetrics { measured_s: 180.0, mean_fps: Some(60.0), low1_fps: Some(55.0), worst_ms: Some(32.0), hitch_count: 1, ..Default::default() };
        assert_eq!(detect_trigger(Some(0), &[], &healthy), None);

        let hitchy =
            LogMetrics { measured_s: 30.0, mean_fps: Some(58.0), low1_fps: Some(40.0), worst_ms: Some(310.0), hitch_count: 2, ..Default::default() };
        assert_eq!(detect_trigger(Some(0), &[], &hitchy), Some("hitch"));
    }

    #[test]
    fn extracts_actual_launcher_ini_keys_and_skips_paths() {
        let ini = Ini::parse("[fefw]\nfps60 = 1\nfps120 = 0\noutput = 1080p\n[dlss]\nupscaler = fsr\nframe_gen = 1\n[fs]\nsave_dir = D:/Secret\n");
        let s = safe_ini_settings(&ini);
        assert_eq!(s.get("fefw.fps60").map(String::as_str), Some("1"));
        assert_eq!(s.get("fefw.fps120").map(String::as_str), Some("0"));
        assert_eq!(s.get("dlss.upscaler").map(String::as_str), Some("fsr"));
        assert_eq!(s.get("dlss.frame_gen").map(String::as_str), Some("1"));
        assert!(!s.contains_key("fs.save_dir"));
    }

    #[test]
    fn rechecks_saved_opt_out_on_disk() {
        let dir = std::env::temp_dir().join(format!("fefw-report-pref-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        assert!(!is_enabled_on_disk(&dir));
        fs::write(dir.join("launcher.json"), r#"{"send_reports": true}"#).unwrap();
        assert!(is_enabled_on_disk(&dir));
        fs::write(dir.join("launcher.json"), r#"{"send_reports": false}"#).unwrap();
        assert!(!is_enabled_on_disk(&dir));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn base64_encodes_ascii_and_utf8() {
        assert_eq!(base64_encode(b""), "");
        assert_eq!(base64_encode(b"f"), "Zg==");
        assert_eq!(base64_encode(b"fo"), "Zm8=");
        assert_eq!(base64_encode(b"foo"), "Zm9v");
        assert_eq!(base64_encode(b"{\"ok\":true}"), "eyJvayI6dHJ1ZX0=");
    }

    #[test]
    fn slices_long_log_keeping_head_and_tail() {
        let log = format!("{}{}", "A".repeat(100), "B".repeat(100));
        let sliced = slice_log(&log, 20, 20);
        assert!(sliced.starts_with(&"A".repeat(20)));
        assert!(sliced.ends_with(&"B".repeat(20)));
        assert!(sliced.contains("trimmed 160 bytes"));
    }
}
