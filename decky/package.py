#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Packs the built plugin into out/fefw-recomp-decky.zip (run `pnpm run build` first)."""

import sys
import zipfile
from pathlib import Path

here = Path(__file__).resolve().parent
files = {name: here / name for name in
         ("dist/index.js", "main.py", "package.json", "plugin.json", "README.md", "py_modules/fefw_launcher.py")}
files["LICENSE"] = here.parent / "LICENSE"
missing = [str(path) for path in files.values() if not path.is_file()]
if missing:
    sys.exit("missing: " + ", ".join(missing))
out = here / "out" / "fefw-recomp-decky.zip"
out.parent.mkdir(exist_ok=True)
with zipfile.ZipFile(out, "w", zipfile.ZIP_DEFLATED) as z:
    for name, path in files.items():
        z.write(path, f"fefw-recomp/{name}")
print(out)
