#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Fetch and inspect anonymized fefw-recomp reports from the Cloudflare Worker."""

from __future__ import annotations

import argparse
import json
import os
import shutil
import subprocess
import sys
import urllib.error
import urllib.parse
import urllib.request
from pathlib import Path


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
        print(f"HTTP {e.code} from {url}: {body}", file=sys.stderr)
        sys.exit(1)
    except urllib.error.URLError as e:
        print(f"network error for {url}: {e}", file=sys.stderr)
        sys.exit(1)


def _decompress_zstd(data: bytes) -> bytes:
    try:
        import zstandard  # type: ignore

        return zstandard.ZstdDecompressor().decompress(data)
    except ImportError:
        pass
    zstd_bin = shutil.which("zstd")
    if zstd_bin:
        proc = subprocess.run(
            [zstd_bin, "-d", "-c"],
            input=data,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            check=False,
        )
        if proc.returncode == 0:
            return proc.stdout
        print(f"zstd failed: {proc.stderr.decode('utf-8', errors='replace')}", file=sys.stderr)
        sys.exit(1)
    print("error: install python 'zstandard' or the 'zstd' CLI to decompress reports", file=sys.stderr)
    sys.exit(1)


def _save_bundle(report_id: str, raw_zst: bytes, out_dir: Path) -> Path:
    bundle = json.loads(_decompress_zstd(raw_zst).decode("utf-8", errors="replace"))
    dest = out_dir / report_id
    dest.mkdir(parents=True, exist_ok=True)
    (dest / "report.json").write_text(json.dumps(bundle, indent=2), encoding="utf-8")
    if bundle.get("log"):
        (dest / "fefw.log").write_text(bundle["log"], encoding="utf-8")
    for name, text in (bundle.get("crash_files") or {}).items():
        safe = Path(name).name
        if safe and text:
            (dest / safe).write_text(text, encoding="utf-8")
    return dest


def _print_table(rows: list[dict]) -> None:
    if not rows:
        print("No reports found.")
        return
    hdr = f"{'DATE':19}  {'TRIGGER':9}  {'OS':7}  {'FPS (MEAN/1%)':13}  {'WORST':8}  {'BUILD':14}  {'GPU':28}  {'ID'}"
    print(hdr)
    print("-" * len(hdr))
    for r in rows:
        dt = (r.get("created_at") or "")[:19].replace("T", " ")
        trig = (r.get("trigger") or "")[:9]
        os_name = (r.get("os") or "")[:7]
        mf, lf = r.get("mean_fps"), r.get("low1_fps")
        fps = f"{mf:.1f}/{lf:.1f}" if mf is not None and lf is not None else "-"
        wm = f"{r['worst_ms']:.0f}ms" if r.get("worst_ms") is not None else "-"
        bld = (r.get("build_id") or "")[:14]
        gpu = (r.get("gpu") or "")[:28]
        rid = r.get("id") or ""
        print(f"{dt:19}  {trig:9}  {os_name:7}  {fps:13}  {wm:8}  {bld:14}  {gpu:28}  {rid}")
        if r.get("crash_cause"):
            print(f"    cause: {r['crash_cause']}")


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
        for r in rows:
            rid = r["id"]
            if (out_dir / rid / "report.json").is_file():
                continue
            raw = _request(f"{base}/v1/reports/{rid}", token)
            _save_bundle(rid, raw, out_dir)
            downloaded += 1
        print(f"Synced {downloaded} new report(s) into {out_dir} ({len(rows)} matched).")
    elif args.cmd == "delete":
        _request(f"{base}/v1/reports/{args.id}", token, method="DELETE")
        print(f"Deleted report {args.id}")


if __name__ == "__main__":
    main()
