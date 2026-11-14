#!/usr/bin/env python3
"""Publish a CI result where the issue bot reads it: <drop>/results/<sha>.json and the Windows build.

Usage: report.py DROP_DIR SHA BRANCH RESULT BUILD_URL LOG_FILE [ARTIFACT_DIR]
"""

import json
import os
import shutil
import sys

drop, sha, branch, result, build_url, log_file = sys.argv[1:7]
artifact_dir = sys.argv[7] if len(sys.argv) > 7 else None

tail = ""
if os.path.exists(log_file):
    with open(log_file, encoding="utf-8", errors="replace") as f:
        tail = "".join(f.readlines()[-200:])

artifacts = []
if result == "SUCCESS" and artifact_dir and os.path.isdir(artifact_dir):
    target = os.path.join(drop, "artifacts", sha)
    os.makedirs(target, exist_ok=True)
    for name in sorted(os.listdir(artifact_dir)):
        shutil.copy2(os.path.join(artifact_dir, name), os.path.join(target, name))
        artifacts.append(os.path.join(target, name))

results = os.path.join(drop, "results")
os.makedirs(results, exist_ok=True)
tmp = os.path.join(results, sha + ".tmp")
with open(tmp, "w", encoding="utf-8") as f:
    json.dump({"sha": sha, "branch": branch, "result": result, "build_url": build_url,
               "artifacts": artifacts, "log_tail": tail}, f, ensure_ascii=False, indent=2)
os.replace(tmp, os.path.join(results, sha + ".json"))

# The bot runs under another account and the agent's umask is restrictive: open the results for reading.
for folder in (results, os.path.join(drop, "artifacts"), os.path.join(drop, "artifacts", sha)):
    if os.path.isdir(folder):
        os.chmod(folder, 0o755)
for path in artifacts + [os.path.join(results, sha + ".json")]:
    os.chmod(path, 0o644)
print(f"CI result {result} for {sha} published to {results}")
