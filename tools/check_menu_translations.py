#!/usr/bin/env python3
"""Fails when an F1 menu table (locales/*.ini) lacks a string the menu shows (CI runs it).

The strings are read from the sources that include nxrt/menu_i18n.h: literals passed to tr() and to the helpers that call
it (HELPERS), the k...Names[] lists and the help texts. A string built some other way, such as one a function returns for
the menu to translate, is not seen, so a clean run does not prove a table is complete.
"""
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
HEADER = '#include "nxrt/menu_i18n.h"'

# function -> the positions of its arguments that the menu translates
HELPERS = {
    "tr": (0,),
    "section": (0,),
    "row": (1, 3),
    "button": (1,),
    "ini_combo_row": (2, 3, 6),
    "path_row": (0, 2),
}

# Shown the same in every language: product names and the on-screen keyboard's rows.
UNTRANSLATED = {"RomFS", "ExeFS", "QWERTYUIOP", "ASDFGHJKL_", "ZXCVBNM;:/"}

TOKEN = re.compile(r'//[^\n]*|/\*.*?\*/|"(?:\\.|[^"\\\n])*"|\'(?:\\.|[^\'\\\n])*\'|[A-Za-z_]\w*|\S', re.S)
ESCAPES = {"n": "\n", "t": "\t", "r": "\r"}


def needs_translation(text):
    if text in UNTRANSLATED or not re.search("[A-Za-z]", text):
        return False
    # a multiplier ("1.5x") or the name of a setting ("[fs] romfs")
    return not re.fullmatch(r"[\d.]+x|\[\w+\] \w+", text)


def literal(tokens):
    """The string of adjacent literals, or None when the tokens are anything else."""
    if not tokens or not all(t.startswith('"') for t in tokens):
        return None
    return re.sub(r"\\(.)", lambda m: ESCAPES.get(m[1], m[1]), "".join(t[1:-1] for t in tokens))


def literals(tokens):
    """The strings of an expression: the whole of it, or either side of a ?: in it."""
    whole = literal(tokens)
    if whole is not None:
        return [whole]
    found = []
    for i, token in enumerate(tokens):
        if token not in "?:":
            continue
        end = i + 1
        while end < len(tokens) and tokens[end].startswith('"'):
            end += 1
        side = literal(tokens[i + 1:end])
        if side is not None and (end == len(tokens) or tokens[end] in (":", ")")):
            found.append(side)
    return found


def split(tokens, start, closers):
    """The comma-separated parts from `start` up to the first of `closers` outside brackets."""
    parts, part, depth = [], [], 0
    for token in tokens[start:]:
        if depth == 0 and token in closers:
            break
        depth += (token in ("(", "[", "{")) - (token in (")", "]", "}"))
        if token == "," and depth == 0:
            parts.append(part)
            part = []
        else:
            part.append(token)
    return parts + [part]


def menu_strings(source):
    """(text, line) of each string to translate, and the helpers that were given one."""
    matches = [m for m in TOKEN.finditer(source) if not m[0].startswith(("//", "/*"))]
    tokens = [m[0] for m in matches]
    found, used = [], set()

    def add(expression, at):
        texts = literals(expression)
        line = source.count("\n", 0, matches[at].start()) + 1
        found.extend((text, line) for text in texts)
        return bool(texts)

    for i, token in enumerate(tokens):
        after = tokens[i + 1:i + 5]
        if token in HELPERS and after[:1] == ["("]:
            args = split(tokens, i + 2, (")",))
            for position in HELPERS[token]:
                if position < len(args) and add(args[position], i):
                    used.add(token)
        elif re.fullmatch(r"k\w*Names", token) and after == ["[", "]", "=", "{"]:
            for item in split(tokens, i + 5, ("}",)):
                add(item, i)
        elif token == "help" and i and tokens[i - 1] == "." and after[:1] == ["="]:
            add(split(tokens, i + 2, (";",))[0], i)
    return found, used


def table_keys(path):
    """The keys with a translation, read as menu_i18n.cpp reads them."""
    keys = set()
    for line in path.read_text(encoding="utf-8-sig").splitlines():
        line = line.strip()
        if not line or line[0] in "#;":
            continue
        key, _, value = line.partition("\t" if "\t" in line else "=")
        if key.strip() and value.strip():
            keys.add(key.strip())
    return keys


def main():
    strings, used = {}, set()
    for path in sorted((ROOT / "runtime").rglob("*.cpp")):
        if "tests" in path.parts:
            continue
        source = path.read_text(encoding="utf-8", errors="replace")
        if HEADER not in source:
            continue
        found, helpers = menu_strings(source)
        used |= helpers
        for text, line in found:
            if needs_translation(text.strip()):
                strings.setdefault(text.strip(), f"{path.relative_to(ROOT).as_posix()}:{line}")

    problems = []
    # a renamed helper would silently drop its strings from the check
    problems += [f"no string found for {name}(): update HELPERS in this script" for name in HELPERS if name not in used]
    tables = sorted((ROOT / "locales").glob("*.ini"))
    if not tables:
        problems.append("no tables found in locales/")
    for table in tables:
        keys = table_keys(table)
        problems += [f"locales/{table.name}: no translation for {text!r} ({where})" for text, where in strings.items()
                     if text not in keys]

    if problems:
        sys.stdout.reconfigure(errors="backslashreplace")
        print("\n".join(problems))
        print(f"\n{len(problems)} translation problem(s). The format of a table is in locales/README.md.")
        return 1
    print(f"F1 menu tables have all {len(strings)} strings this check finds.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
