#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Fetch and inspect anonymized fefw-recomp reports from the Cloudflare Worker."""

from __future__ import annotations

import argparse
import io
import json
import os
import re
import shutil
import subprocess
import sys
import threading
import urllib.error
import urllib.parse
import urllib.request
from pathlib import Path

# Hard cap on expanded JSON size to protect against maliciously crafted zstd bombs.
MAX_EXPANDED_BYTES = 8 * 1024 * 1024  # 8 MiB


def _env_or_die(val: str | None, env_name: str) -> str:
    v = (val or os.environ.get(env_name, "")).strip()
    if not v:
        print(f"error: set {env_name} or pass the corresponding flag", file=sys.stderr)
        sys.exit(2)
    return v


def _request(url: str, token: str, method: str = "GET") -> bytes:
    req = urllib.request.Request(
        url,
        method=method,
        headers={
            "Authorization": f"Bearer {token}",
            "User-Agent": "fefw-fetch-reports",
        },
    )
    try:
        with urllib.request.urlopen(req, timeout=30) as resp:
            return resp.read()
    except urllib.error.HTTPError as e:
        body = e.read().decode("utf-8", errors="replace").strip()
        raise RuntimeError(f"HTTP {e.code} from {url}: {body}") from e
    except urllib.error.URLError as e:
        raise RuntimeError(f"network error for {url}: {e}") from e


def _read_bounded(stream: io.BufferedIOBase, limit: int) -> bytes:
    out = bytearray()
    while True:
        chunk = stream.read(min(65536, limit + 1 - len(out)))
        if not chunk:
            break
        out.extend(chunk)
        if len(out) > limit:
            raise ValueError(f"decompressed report exceeds {limit} bytes")
    return bytes(out)


def _decompress_zstd(data: bytes, max_bytes: int = MAX_EXPANDED_BYTES) -> bytes:
    try:
        import zstandard  # type: ignore

        dctx = zstandard.ZstdDecompressor()
        with dctx.stream_reader(io.BytesIO(data)) as reader:
            return _read_bounded(reader, max_bytes)
    except ImportError:
        pass
    zstd_bin = shutil.which("zstd")
    if zstd_bin:
        proc = subprocess.Popen(
            [zstd_bin, "-d", "-c"],
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
        )
        assert proc.stdin is not None and proc.stdout is not None and proc.stderr is not None

        def _feed() -> None:
            try:
                proc.stdin.write(data)
            except BrokenPipeError:
                pass
            finally:
                try:
                    proc.stdin.close()
                except OSError:
                    pass

        feeder = threading.Thread(target=_feed, daemon=True)
        feeder.start()
        try:
            out = _read_bounded(proc.stdout, max_bytes)
        except ValueError:
            proc.kill()
            proc.wait()
            raise
        finally:
            proc.stdout.close()
            feeder.join(timeout=5)
        stderr = proc.stderr.read().decode("utf-8", errors="replace").strip()
        proc.stderr.close()
        if proc.wait() != 0:
            raise RuntimeError(f"zstd failed: {stderr}")
        return out
    raise RuntimeError("install python 'zstandard' or the 'zstd' CLI to decompress reports")


def _save_bundle(report_id: str, raw_zst: bytes, out_dir: Path) -> Path:
    bundle = json.loads(_decompress_zstd(raw_zst).decode("utf-8", errors="replace"))
    if not isinstance(bundle, dict):
        raise ValueError("report payload is not a JSON object")
    dest = out_dir / report_id
    dest.mkdir(parents=True, exist_ok=True)
    (dest / "report.json").write_text(json.dumps(bundle, indent=2), encoding="utf-8")
    if isinstance(bundle.get("log"), str) and bundle["log"]:
        (dest / "fefw.log").write_text(bundle["log"], encoding="utf-8")
    crash_files = bundle.get("crash_files")
    if isinstance(crash_files, dict):
        for name, text in crash_files.items():
            safe = Path(str(name)).name
            if safe and isinstance(text, str) and text:
                (dest / safe).write_text(text, encoding="utf-8")
    return dest


_CTRL_RE = re.compile(r"[\x00-\x1f\x7f-\x9f]+")


def _safe_term(val: object, max_len: int | None = None) -> str:
    s = _CTRL_RE.sub(" ", str(val or "")).strip()
    return s[:max_len] if max_len is not None else s


def _print_table(rows: list[dict]) -> None:
    if not rows:
        print("No reports found.")
        return
    hdr = f"{'DATE':19}  {'TRIGGER':9}  {'OS':7}  {'FPS (MEAN/1%)':13}  {'WORST':8}  {'BUILD':14}  {'GPU':28}  {'ID'}"
    print(hdr)
    print("-" * len(hdr))
    for r in rows:
        dt = _safe_term(r.get("created_at"), 19).replace("T", " ")
        trig = _safe_term(r.get("trigger"), 9)
        os_name = _safe_term(r.get("os"), 7)
        mf, lf = r.get("mean_fps"), r.get("low1_fps")
        fps = f"{mf:.1f}/{lf:.1f}" if mf is not None and lf is not None else "-"
        wm = f"{r['worst_ms']:.0f}ms" if r.get("worst_ms") is not None else "-"
        bld = _safe_term(r.get("build_id"), 14)
        gpu = _safe_term(r.get("gpu"), 28)
        rid = _safe_term(r.get("id"))
        print(f"{dt:19}  {trig:9}  {os_name:7}  {fps:13}  {wm:8}  {bld:14}  {gpu:28}  {rid}")
        if r.get("crash_cause"):
            print(f"    cause: {_safe_term(r['crash_cause'])}")


def _build_list_url(base: str, args: argparse.Namespace) -> str:
    params: dict[str, str] = {"limit": str(args.limit), "offset": str(args.offset)}
    for key in ("trigger", "build_id", "gpu", "os", "since"):
        val = getattr(args, key, None)
        if val:
            params[key] = val
    return f"{base.rstrip('/')}/v1/reports?{urllib.parse.urlencode(params)}"


def main() -> None:
    p = argparse.ArgumentParser(description="Fetch anonymized fefw-recomp reports.")
    p.add_argument("--url", help="Worker base URL (or FEFW_REPORT_URL)")
    p.add_argument("--token", help="Admin bearer token (or FEFW_REPORT_ADMIN_TOKEN)")
    sub = p.add_subparsers(dest="cmd", required=True)

    for name in ("list", "sync"):
        sp = sub.add_parser(name, help="List reports" if name == "list" else "Download new reports locally")
        sp.add_argument("--trigger", choices=["crash", "freeze", "fatal", "exit_code", "hitch", "low_fps"])
        sp.add_argument("--build-id", dest="build_id")
        sp.add_argument("--gpu")
        sp.add_argument("--os")
        sp.add_argument("--since", help="ISO-8601 UTC timestamp, e.g. 2026-11-01T00:00:00Z")
        sp.add_argument("--limit", type=int, default=30)
        sp.add_argument("--offset", type=int, default=0)
        if name == "sync":
            sp.add_argument("--out", default="work/reports", help="Output directory (default: work/reports)")

    sp_get = sub.add_parser("get", help="Download and unpack a single report by ID")
    sp_get.add_argument("id")
    sp_get.add_argument("--out", default="work/reports", help="Output directory (default: work/reports)")

    sp_del = sub.add_parser("delete", help="Delete a report by ID from R2 and D1")
    sp_del.add_argument("id")

    args = p.parse_args()
    base = _env_or_die(args.url, "FEFW_REPORT_URL").rstrip("/")
    token = _env_or_die(args.token, "FEFW_REPORT_ADMIN_TOKEN")

    try:
        if args.cmd == "list":
            data = json.loads(_request(_build_list_url(base, args), token))
            _print_table(data.get("reports") or [])
        elif args.cmd == "get":
            raw = _request(f"{base}/v1/reports/{args.id}", token)
            dest = _save_bundle(args.id, raw, Path(args.out))
            print(f"Saved report to {dest}")
        elif args.cmd == "sync":
            out_dir = Path(args.out)
            data = json.loads(_request(_build_list_url(base, args), token))
            rows = data.get("reports") or []
            downloaded = 0
            failed = 0
            for r in rows:
                rid = r.get("id")
                if not rid or (out_dir / rid / "report.json").is_file():
                    continue
                try:
                    raw = _request(f"{base}/v1/reports/{rid}", token)
                    _save_bundle(rid, raw, out_dir)
                    downloaded += 1
                except Exception as e:
                    failed += 1
                    print(f"warning: skipping report {rid}: {e}", file=sys.stderr)
            msg = f"Synced {downloaded} new report(s) into {out_dir} ({len(rows)} matched"
            if failed:
                msg += f", {failed} failed"
            print(msg + ").")
        elif args.cmd == "delete":
            _request(f"{base}/v1/reports/{args.id}", token, method="DELETE")
            print(f"Deleted report {args.id}")
    except Exception as e:
        print(f"error: {e}", file=sys.stderr)
        sys.exit(1)


if __name__ == "__main__":
    main()
