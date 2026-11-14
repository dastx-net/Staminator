#!/usr/bin/env python3
"""Linux twin of scripts/check-utf8.ps1: project text files must be valid UTF-8 without NUL bytes."""

import os
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
EXCLUDED = ("src/stamina", "src/stamina_setup", "target", ".git", "ci-out", ".stamibot")
NAMES = {"Cargo.lock", ".gitignore", ".gitattributes", ".editorconfig"}
EXTENSIONS = {
    ".bat", ".c", ".cfg", ".cpp", ".css", ".csv", ".h", ".html", ".ini", ".js", ".json", ".md",
    ".ps1", ".rc", ".rs", ".sh", ".toml", ".txt", ".xml", ".yaml", ".yml",
}

bad = []
for folder, dirs, files in os.walk(ROOT):
    rel = os.path.relpath(folder, ROOT).replace(os.sep, "/")
    dirs[:] = [d for d in dirs if (d if rel == "." else f"{rel}/{d}") not in EXCLUDED]
    for name in files:
        if name not in NAMES and os.path.splitext(name)[1].lower() not in EXTENSIONS:
            continue
        path = os.path.join(folder, name)
        with open(path, "rb") as f:
            data = f.read()
        try:
            if b"\0" in data:
                raise ValueError("NUL byte")
            data.decode("utf-8")
        except ValueError:
            bad.append(os.path.relpath(path, ROOT))

if bad:
    print("Not valid UTF-8:")
    for p in sorted(bad):
        print("  " + p)
    sys.exit(1)
print("UTF-8 check passed.")
