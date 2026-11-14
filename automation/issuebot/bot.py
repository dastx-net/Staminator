#!/usr/bin/env python3
"""Staminator issue bot.

Polls GitHub for issues assigned to the bot's account by a trusted initiator and
drives each one through plan -> plan review -> implementation -> implementation
review -> CI -> pull request with Codex. See automation/README.md.

Standard library only: runs under an unprivileged account without extra packages.
"""

import base64
import fcntl
import glob
import json
import os
import re
import subprocess
import sys
import time
import traceback
import urllib.error
import urllib.parse
import urllib.request
from pathlib import Path

HERE = Path(__file__).resolve().parent
MARK = "<!-- stamibot -->"
TERMINAL = {"done", "blocked", "ignored", "cancelled"}

DEFAULTS = {
    "repo": "dastx-net/Staminator",
    "assignee": "losinkatorf",
    "initiators": ["DanielSanIT", "losinkatorf"],
    "base_branch": "master",
    "impl_model": "gpt-6-sol",
    "impl_effort": "high",
    "review_model": "gpt-6-astra",
    "review_effort": "high",
    "probe_model": "gpt-6-luna",
    "budget_limit_percent": 75,
    "budget_max_age_minutes": 30,
    # 0 = no cap: review rounds repeat until no P0/P1 finding is left (the usage limit still applies).
    "max_review_rounds": 0,
    "max_ci_rounds": 3,
    "stage_timeout_minutes": 120,
    "ci_results_dir": "/srv/data/staminator-ci/results",
    "ci_timeout_minutes": 90,
    # End-to-end check of the Windows build in the test VM (automation/e2e/vm.py). Skipped when the VM is busy or off.
    "e2e_enabled": True,
    # How long a finished issue is still watched for new comments that start a follow-up round.
    "followup_days": 14,
    "proxy": "http://127.0.0.1:18888",
    # Hosts that answer directly; everything else goes through the A&D proxy (same split as devuw).
    "no_proxy": "localhost,127.0.0.1,::1,github.com,api.github.com,objects.githubusercontent.com,"
    "release-assets.githubusercontent.com,openai.com,.openai.com,chatgpt.com,.chatgpt.com,"
    "crates.io,.crates.io,static.rust-lang.org",
    "root": "~/stamibot",
    "token_file": "~/.config/stamibot/gh-token",
    "git_name": "stamibot",
    "git_email": "losinkatorf@users.noreply.github.com",
    # Paths people change: the bot's own code and prompts, CI, GitHub workflows, cargo configuration.
    # A branch that touches them is pushed only after an initiator comments `/retry allow-protected`.
    "protected_paths": ["automation/", ".github/", ".cargo/"],
}


class Paused(Exception):
    """Work must wait (budget); retried on a later tick."""


class StageError(Exception):
    pass


def log(*parts):
    print(time.strftime("%Y-%m-%d %H:%M:%S"), *parts, flush=True)


def load_config():
    cfg = dict(DEFAULTS)
    path = Path(os.environ.get("STAMIBOT_CONFIG", "~/.config/stamibot/config.json")).expanduser()
    if path.exists():
        cfg.update(json.loads(path.read_text(encoding="utf-8")))
    cfg["root"] = Path(cfg["root"]).expanduser()
    cfg["token_file"] = Path(cfg["token_file"]).expanduser()
    return cfg


CFG = load_config()
ROOT = CFG["root"]
REPO_DIR = ROOT / "repo"
STATE_DIR = ROOT / "state"
_OPENER = urllib.request.build_opener(urllib.request.ProxyHandler({}))


def token():
    return CFG["token_file"].read_text(encoding="utf-8").strip()


# ---------------------------------------------------------------- GitHub


def gh(method, path, body=None, params=None):
    url = "https://api.github.com" + path
    if params:
        url += "?" + urllib.parse.urlencode(params)
    data = json.dumps(body).encode() if body is not None else None
    req = urllib.request.Request(url, data=data, method=method)
    req.add_header("Authorization", "Bearer " + token())
    req.add_header("Accept", "application/vnd.github+json")
    req.add_header("X-GitHub-Api-Version", "2022-11-28")
    req.add_header("User-Agent", "stamibot")
    with _OPENER.open(req, timeout=30) as resp:
        raw = resp.read()
    return json.loads(raw) if raw else None


def repo_path(suffix=""):
    return "/repos/" + CFG["repo"] + suffix


def post_comment(number, text):
    body = text.strip()
    if len(body) > 60000:
        body = body[:60000] + "\n\n_(truncated)_"
    res = gh("POST", repo_path(f"/issues/{number}/comments"), {"body": body + "\n\n" + MARK})
    return res["id"]


def human_comments(number, after_id=0):
    """Comments written by a trusted initiator by hand (bot comments carry MARK)."""
    items = gh("GET", repo_path(f"/issues/{number}/comments"), params={"per_page": 100})
    out = []
    for c in items:
        if c["id"] <= after_id or MARK in (c.get("body") or ""):
            continue
        if c["user"]["login"] in CFG["initiators"]:
            out.append(c)
    return out


def assigned_by(number):
    events = gh("GET", repo_path(f"/issues/{number}/events"), params={"per_page": 100})
    who = None
    for e in events:
        if e.get("event") == "assigned" and (e.get("assignee") or {}).get("login") == CFG["assignee"]:
            who = (e.get("assigner") or e.get("actor") or {}).get("login")
    return who


# ---------------------------------------------------------------- state


def state_path(number):
    return STATE_DIR / f"issue-{number}" / "state.json"


def load_state(number):
    p = state_path(number)
    return json.loads(p.read_text(encoding="utf-8")) if p.exists() else None


def save_state(st):
    p = state_path(st["number"])
    p.parent.mkdir(parents=True, exist_ok=True)
    tmp = p.with_suffix(".tmp")
    tmp.write_text(json.dumps(st, ensure_ascii=False, indent=2), encoding="utf-8")
    tmp.replace(p)


def all_states():
    out = []
    for p in sorted(STATE_DIR.glob("issue-*/state.json")):
        out.append(json.loads(p.read_text(encoding="utf-8")))
    return out


def set_stage(st, stage, feedback=None):
    log(f"#{st['number']}: {st['stage']} -> {stage}")
    st["stage"] = stage
    st["feedback"] = feedback
    if stage == "done":
        st["done_at"] = time.time()
    save_state(st)


def block(st, reason):
    st["blocked_from"] = st["stage"]
    st["blocked_at_comment"] = post_comment(
        st["number"],
        f"**Stamibot stopped** at stage `{st['stage']}`.\n\n{reason}\n\n"
        "Comment `/retry` (optionally followed by instructions) to continue from this stage.",
    )
    set_stage(st, "blocked")


# ---------------------------------------------------------------- git


def run(cmd, cwd=None, check=True, env=None, timeout=600):
    res = subprocess.run(cmd, cwd=cwd, env=env, text=True, capture_output=True, timeout=timeout)
    if check and res.returncode != 0:
        raise StageError(f"{' '.join(map(str, cmd[:4]))}... failed: {res.stderr.strip()[-2000:]}")
    return res


def git(*args, cwd=None, check=True, auth=False):
    env = None
    if auth:
        # Through the environment, not the command line: other local accounts can read process arguments.
        basic = base64.b64encode(("x-access-token:" + token()).encode()).decode()
        env = dict(os.environ, GIT_CONFIG_COUNT="2",
                   GIT_CONFIG_KEY_0="credential.helper", GIT_CONFIG_VALUE_0="",
                   GIT_CONFIG_KEY_1="http.https://github.com/.extraheader",
                   GIT_CONFIG_VALUE_1=f"Authorization: Basic {basic}")
    return run(["git", *args], cwd=cwd or REPO_DIR, check=check, env=env)


def ensure_repo():
    if not REPO_DIR.exists():
        ROOT.mkdir(parents=True, exist_ok=True)
        run(["git", "clone", f"https://github.com/{CFG['repo']}.git", str(REPO_DIR)], timeout=1800)
        git("config", "user.name", CFG["git_name"])
        git("config", "user.email", CFG["git_email"])
    exclude = REPO_DIR / ".git" / "info" / "exclude"
    if ".stamibot/" not in exclude.read_text(encoding="utf-8"):
        with exclude.open("a", encoding="utf-8") as f:
            f.write("\n.stamibot/\n")
    git("fetch", "--prune", "origin")


def worktree(st):
    return ROOT / "wt" / f"issue-{st['number']}"


def ensure_worktree(st):
    wt = worktree(st)
    if not wt.exists():
        ensure_repo()
        slug = re.sub(r"[^a-z0-9]+", "-", st["issue"]["title"].lower()).strip("-")[:40].strip("-")
        st["branch"] = f"issue-{st['number']}" + (f"-{slug}" if slug else "")
        wt.parent.mkdir(parents=True, exist_ok=True)
        git("worktree", "add", "-B", st["branch"], str(wt), "origin/" + CFG["base_branch"])
        save_state(st)
    (wt / ".stamibot").mkdir(exist_ok=True)
    return wt


def write_issue_file(st):
    wt = ensure_worktree(st)
    issue = st["issue"]
    parts = [f"# Issue #{st['number']}: {issue['title']}", f"Author: {issue['author']}", "", issue.get("body") or "_(no description)_"]
    for c in human_comments(st["number"]):
        parts += ["", f"## Comment by {c['user']['login']} at {c['created_at']}", "", c.get("body") or ""]
    (wt / ".stamibot" / "issue.md").write_text("\n".join(parts) + "\n", encoding="utf-8")


def commit_if_dirty(st, message):
    wt = worktree(st)
    if not git("status", "--porcelain", cwd=wt).stdout.strip():
        return False
    git("add", "-A", cwd=wt)
    msg = (message or "").strip() or f"Work on issue #{st['number']}"
    git("commit", "-m", msg, cwd=wt)
    return True


def head_sha(st):
    return git("rev-parse", "HEAD", cwd=worktree(st)).stdout.strip()


def protected_changes(st):
    """Files of the branch under `protected_paths`, compared with the base branch."""
    out = git("diff", "--name-only", "--no-renames", f"origin/{CFG['base_branch']}...HEAD", cwd=worktree(st)).stdout
    return sorted(p for p in out.splitlines()
                  if any(p == x.rstrip("/") or p.startswith(x) for x in CFG["protected_paths"]))


def commits_ahead(st):
    out = git("rev-list", "--count", f"origin/{CFG['base_branch']}..HEAD", cwd=worktree(st)).stdout.strip()
    return int(out or 0)


# ---------------------------------------------------------------- codex


def codex_env():
    env = dict(os.environ)
    env["PATH"] = f"{Path.home()}/.local/bin:{Path.home()}/.cargo/bin:" + env.get("PATH", "")
    for name in ("HTTPS_PROXY", "HTTP_PROXY", "https_proxy", "http_proxy"):
        env[name] = CFG["proxy"]
    env["NO_PROXY"] = env["no_proxy"] = CFG["no_proxy"]
    return env


def read_budget():
    """Newest rate_limits reading Codex left in its rollouts: (limits, unix time) or (None, 0)."""
    files = glob.glob(str(Path.home() / ".codex/sessions/*/*/*/rollout-*.jsonl"))
    files.sort(key=os.path.getmtime, reverse=True)
    key = '"rate_limits":'
    for f in files[:8]:
        with open(f, "rb") as fh:
            fh.seek(0, 2)
            fh.seek(max(0, fh.tell() - 400_000))
            data = fh.read().decode("utf-8", "replace")
        end = len(data)
        while True:
            i = data.rfind(key, 0, end)
            if i < 0:
                break
            try:
                obj, _ = json.JSONDecoder().raw_decode(data[i + len(key):].lstrip())
            except ValueError:
                obj = None
            if isinstance(obj, dict) and (obj.get("primary") or obj.get("secondary")):
                return obj, os.path.getmtime(f)
            end = i
    return None, 0


def used_percent(limits):
    worst = 0.0
    for name in ("primary", "secondary"):
        w = limits.get(name)
        if not w:
            continue
        resets = w.get("resets_at")
        if resets and resets < time.time():
            continue  # the window has reset since this reading
        worst = max(worst, float(w.get("used_percent") or 0))
    return worst


def check_budget():
    limits, ts = read_budget()
    if limits is None or time.time() - ts > CFG["budget_max_age_minutes"] * 60:
        STATE_DIR.mkdir(parents=True, exist_ok=True)
        subprocess.run(
            ["codex", "exec", "-m", CFG["probe_model"], "-c", 'model_reasoning_effort="low"', "-s", "read-only",
             "--skip-git-repo-check", "-C", str(STATE_DIR), "Reply with the single word OK."],
            env=codex_env(), capture_output=True, text=True, timeout=300,
        )
        limits, ts = read_budget()
    if limits is None:
        raise Paused("cannot read Codex usage (is `codex login` done for this account?)")
    used = used_percent(limits)
    if used >= CFG["budget_limit_percent"]:
        raise Paused(f"Codex usage is {used:.0f}%, the limit for automation is {CFG['budget_limit_percent']}%")
    return used


def codex_turn(st, role, prompt, schema, sandbox):
    """One Codex turn in the issue worktree. `role` keys the session, so later turns resume it."""
    used = check_budget()
    wt = ensure_worktree(st)
    reviewer = role.endswith("review")
    model = CFG["review_model"] if reviewer else CFG["impl_model"]
    effort = CFG["review_effort"] if reviewer else CFG["impl_effort"]
    logs = state_path(st["number"]).parent / "logs"
    logs.mkdir(parents=True, exist_ok=True)
    stamp = time.strftime("%Y%m%d-%H%M%S")
    out_file = logs / f"{stamp}-{role}.json"
    sid = st["sessions"].get(role)
    cmd = ["codex", "exec"] + (["resume", sid] if sid else [])
    cmd += [
        "-m", model,
        "-c", f'model_reasoning_effort="{effort}"',
        "-c", 'approval_policy="never"',
        "-c", f'sandbox_mode="{sandbox}"',
        "-c", "sandbox_workspace_write.network_access=true",
        "-c", f'sandbox_workspace_write.writable_roots=["{Path.home()}/.cargo"]',
        "--json", "-o", str(out_file), "--output-schema", str(HERE / "schemas" / schema),
        prompt,
    ]
    log(f"#{st['number']}: codex {role} ({model}, {'resume' if sid else 'new'}), usage {used:.0f}%")
    try:
        res = subprocess.run(cmd, cwd=wt, env=codex_env(), text=True, capture_output=True,
                             timeout=CFG["stage_timeout_minutes"] * 60)
    except subprocess.TimeoutExpired:
        raise StageError(f"Codex {role} turn exceeded {CFG['stage_timeout_minutes']} minutes")
    (logs / f"{stamp}-{role}.jsonl").write_text(res.stdout, encoding="utf-8")
    if not sid:
        m = re.search(r'"(?:thread_id|session_id)"\s*:\s*"([0-9a-f-]{36})"', res.stdout)
        if m:
            st["sessions"][role] = m.group(1)
            save_state(st)
    if res.returncode != 0 or not out_file.exists():
        raise StageError(f"Codex {role} turn failed (exit {res.returncode}): {(res.stderr or res.stdout).strip()[-1500:]}")
    try:
        return json.loads(out_file.read_text(encoding="utf-8"))
    except ValueError:
        raise StageError(f"Codex {role} turn returned no JSON: {out_file.read_text(encoding='utf-8')[:500]}")


def prompt_text(name, st, **extra):
    text = (HERE / "prompts" / name).read_text(encoding="utf-8")
    values = {"number": st["number"], "title": st["issue"]["title"], "base": CFG["base_branch"]}
    values.update(extra)
    for k, v in values.items():
        text = text.replace("{" + k + "}", str(v))
    return text


def turn(st, role, first_prompt, schema, sandbox):
    """First turn gets the stage prompt; a resumed session gets only the pending feedback."""
    write_issue_file(st)
    feedback = st.get("feedback")
    if st["sessions"].get(role) and st.get("stage_started") == st["stage"]:
        prompt = feedback or "Continue."
    else:
        prompt = prompt_text(first_prompt, st)
        if feedback:
            prompt += "\n\n## Input for this turn\n\n" + feedback
    st["stage_started"] = st["stage"]
    result = codex_turn(st, role, prompt, schema, sandbox)
    st["feedback"] = None
    save_state(st)
    return result


# ---------------------------------------------------------------- stages


STAGE_LABEL = {"plan": "planning", "plan_review": "plan review", "implement": "implementation",
               "impl_review": "implementation review"}


def ask(st, questions):
    lines = [f"**Stamibot has questions** (stage: {STAGE_LABEL.get(st['stage'], st['stage'])})", ""]
    lines += [f"{i}. {q}" for i, q in enumerate(questions, 1)]
    lines += ["", "Reply in a comment on this issue. Work resumes automatically after the answer."]
    target = st.get("pr_number") or st["number"]
    cid = post_comment(target, "\n".join(lines).replace("on this issue", "here"))
    st["waiting"] = {"comment_id": cid, "questions": questions, "target": target}
    save_state(st)
    log(f"#{st['number']}: waiting for answers")


def take_answers(st):
    """Move the initiator's replies into feedback. False while there are none."""
    w = st["waiting"]
    target = w.get("target") or st["number"]
    answers = human_comments(target, after_id=w["comment_id"])
    if not answers:
        return False
    if st.get("pr_seen"):  # answers are not review feedback: move the watermark past them
        key = "conv" if target == st.get("pr_number") else "issue"
        st["pr_seen"][key] = max(st["pr_seen"][key], max(c["id"] for c in answers))
    text = "\n\n".join(f"{c['user']['login']}: {c['body']}" for c in answers)
    qa = worktree(st) / ".stamibot" / "qa.md"
    with qa.open("a", encoding="utf-8") as f:
        f.write("## Questions\n\n" + "\n".join(f"- {q}" for q in w["questions"]) + "\n\n## Answers\n\n" + text + "\n\n")
    st["waiting"] = None
    st["feedback"] = "Answers from the initiator to your questions (also saved in .stamibot/qa.md):\n\n" + text
    save_state(st)
    return True


def findings_text(findings):
    return "\n".join(f"- [{f['severity']}] {f['title']} ({f['location']}): {f['details']}" for f in findings) or "- none"


def needs_revision(review):
    """Only findings worse than P2 send the work back; P2 and P3 are reported and left open."""
    return any(f["severity"] in ("P0", "P1") for f in review["findings"])


def stage_plan(st):
    r = turn(st, "impl", "plan.md", "work.json", "workspace-write")
    if r["status"] == "questions" and r["questions"]:
        return ask(st, r["questions"])
    if r["status"] == "blocked":
        return block(st, r["summary"])
    if not (worktree(st) / ".stamibot" / "plan.md").exists():
        raise StageError("planning turn did not write .stamibot/plan.md")
    set_stage(st, "plan_review", "The plan in .stamibot/plan.md was updated. Review it again.")


def stage_plan_review(st):
    r = turn(st, "plan_review", "plan_review.md", "review.json", "read-only")
    if r["verdict"] == "questions" and r["questions"]:
        return ask(st, r["questions"])
    if needs_revision(r):
        st["rounds"]["plan_review"] += 1
        if 0 < CFG["max_review_rounds"] < st["rounds"]["plan_review"]:
            return block(st, "The plan did not pass review after "
                         f"{CFG['max_review_rounds']} rounds. Open findings:\n\n{findings_text(r['findings'])}")
        return set_stage(st, "plan", "The plan reviewer asks for changes. Update .stamibot/plan.md.\n\n"
                         + findings_text(r["findings"]))
    plan = (worktree(st) / ".stamibot" / "plan.md").read_text(encoding="utf-8")
    post_comment(st["number"], f"**Plan (reviewed, approved)**\n\n{plan}\n\n"
                 f"Review notes:\n\n{findings_text(r['findings'])}\n\nImplementation starts now.")
    st["stage_started"] = None  # the implementer session gets the full implementation prompt
    set_stage(st, "implement")


def stage_implement(st):
    r = turn(st, "impl", "implement.md", "work.json", "workspace-write")
    if r["status"] == "questions" and r["questions"]:
        return ask(st, r["questions"])
    if r["status"] == "blocked":
        return block(st, r["summary"])
    commit_if_dirty(st, r["commit_message"])
    if commits_ahead(st) == 0:
        return block(st, "The implementation turn produced no changes.\n\n" + r["summary"])
    st["last_summary"] = r["summary"]
    st["pr_title"] = r["pr_title"] or st.get("pr_title") or st["issue"]["title"]
    st["pr_body"] = r["pr_body"] or st.get("pr_body") or r["summary"]
    set_stage(st, "impl_review", "The implementation changed (new commit on the branch). Review it again.")


def stage_impl_review(st):
    r = turn(st, "impl_review", "impl_review.md", "review.json", "read-only")
    if r["verdict"] == "questions" and r["questions"]:
        return ask(st, r["questions"])
    st["review_notes"] = findings_text(r["findings"])
    if needs_revision(r):
        st["rounds"]["impl_review"] += 1
        if 0 < CFG["max_review_rounds"] < st["rounds"]["impl_review"]:
            return block(st, "The implementation did not pass review after "
                         f"{CFG['max_review_rounds']} rounds. Open findings:\n\n{st['review_notes']}")
        return set_stage(st, "implement", "The code reviewer asks for changes. Fix them.\n\n" + st["review_notes"])
    set_stage(st, "ci_push")


def stage_ci_push(st):
    touched = protected_changes(st)
    if touched and st.get("protected_ok") != head_sha(st):
        block(st, "The branch changes protected paths, which only people change:\n\n"
              + "\n".join(f"- `{p}`" for p in touched)
              + "\n\nNothing was pushed. Comment `/retry allow-protected` to push this commit as it is. "
              "Any other `/retry` sends the work back to implementation to undo these changes.")
        st["protected_block"] = touched
        save_state(st)
        return
    git("push", "--force-with-lease", "origin", f"HEAD:refs/heads/{st['branch']}", cwd=worktree(st), auth=True)
    st["ci"] = {"sha": head_sha(st), "since": time.time()}
    set_stage(st, "ci_wait" if CFG["ci_results_dir"] else "pr")


def stage_ci_wait(st):
    """True when the stage changed, False while the CI result is not there yet."""
    result = Path(CFG["ci_results_dir"]) / f"{st['ci']['sha']}.json"
    if not result.exists():
        if time.time() - st["ci"]["since"] > CFG["ci_timeout_minutes"] * 60:
            block(st, f"No CI result for `{st['ci']['sha'][:12]}` after {CFG['ci_timeout_minutes']} minutes.")
            return True
        return False
    ci = json.loads(result.read_text(encoding="utf-8"))
    st["ci"]["url"] = ci.get("build_url")
    st["ci"]["artifacts"] = ci.get("artifacts") or []
    if ci.get("result") == "SUCCESS":
        set_stage(st, "e2e" if CFG["e2e_enabled"] else "pr")
        return True
    st["rounds"]["ci"] += 1
    if st["rounds"]["ci"] > CFG["max_ci_rounds"]:
        block(st, f"CI still fails after {CFG['max_ci_rounds']} fix rounds: {ci.get('build_url')}")
        return True
    set_stage(st, "implement", "The CI build of your branch failed. Fix it. Log tail:\n\n```\n"
              + (ci.get("log_tail") or "")[-15000:] + "\n```")
    return True


VM_TOOL = HERE.parent / "e2e" / "vm.py"


def vm_status():
    res = subprocess.run([sys.executable, str(VM_TOOL), "status"], text=True, capture_output=True, timeout=120)
    return dict(pair.split("=", 1) for pair in res.stdout.split() if "=" in pair)


def stage_e2e(st):
    """Codex runs the Windows build in the test VM and looks at it. Never blocks on an unavailable VM."""
    exe = next((a for a in st["ci"].get("artifacts", []) if a.endswith(".exe")), None)
    vm = vm_status() if exe else {}
    reason = None
    if not exe:
        reason = "CI published no Windows build"
    elif vm.get("ssh") != "True" or vm.get("vnc") != "True":
        reason = "the test VM is not reachable"
    elif vm.get("session") in (None, "None"):
        reason = "the test user is not logged in to the test VM"
    elif vm.get("rdp") == "True":
        reason = "the test VM is in use over RDP"
    elif subprocess.run([sys.executable, str(VM_TOOL), "console"], capture_output=True, timeout=180).returncode != 0:
        reason = "could not put the test desktop on the VM console"
    if reason:
        st["e2e_notes"] = "skipped: " + reason
        log(f"#{st['number']}: e2e {st['e2e_notes']}")
        return set_stage(st, "pr")
    st["sessions"].pop("e2e", None)  # every check is a fresh session
    write_issue_file(st)
    r = codex_turn(st, "e2e", prompt_text("e2e.md", st, exe=exe, vm=f"python3 {VM_TOOL}",
                                          summary=st.get("last_summary") or ""), "e2e.json", "workspace-write")
    st["e2e_notes"] = f"{r['status']}: {r['summary']}"
    if r["status"] == "fail":
        st["rounds"]["ci"] += 1
        if st["rounds"]["ci"] > CFG["max_ci_rounds"]:
            return block(st, f"The end-to-end check still fails after {CFG['max_ci_rounds']} fix rounds.\n\n"
                         + st["e2e_notes"])
        return set_stage(st, "implement", "The end-to-end check of the Windows build in the test VM failed. "
                         "Fix the problems.\n\n" + r["summary"] + "\n\n" + "\n".join(f"- {p}" for p in r["problems"]))
    set_stage(st, "pr")


def stage_pr(st):
    if st.get("pr_number"):
        post_comment(st["pr_number"], f"**Stamibot addressed the feedback** (head `{head_sha(st)[:12]}`).\n\n"
                     f"{st.get('last_summary') or ''}\n\nReview notes left open (P2 and below):\n\n"
                     f"{st.get('review_notes') or '- none'}\n\nEnd-to-end check: {st.get('e2e_notes') or 'not run'}")
        return set_stage(st, "pr_open")
    link = "Follow-up to" if st.get("followup") else "Closes"
    body = (f"{st['pr_body'].strip()}\n\n{link} #{st['number']}\n\n---\n"
            f"Automated by stamibot: implementation `{CFG['impl_model']}`, review `{CFG['review_model']}`.\n\n"
            f"Review notes left open (P2 and below):\n\n{st.get('review_notes') or '- none'}\n")
    if st.get("ci", {}).get("url"):
        body += f"\nCI: {st['ci']['url']}\n"
    body += f"\nEnd-to-end check in the Windows VM: {st.get('e2e_notes') or 'not run'}\n"
    try:
        pr = gh("POST", repo_path("/pulls"), {"title": st["pr_title"], "head": st["branch"],
                                               "base": CFG["base_branch"], "body": body})
    except urllib.error.HTTPError as e:
        if e.code != 422:
            raise
        owner = CFG["repo"].split("/")[0]
        found = gh("GET", repo_path("/pulls"), params={"head": f"{owner}:{st['branch']}", "state": "open"})
        if not found:
            raise StageError("GitHub refused the pull request: " + e.read().decode("utf-8", "replace")[:500])
        pr = found[0]
    st["pr_url"] = pr["html_url"]
    st["pr_number"] = pr["number"]
    cid = post_comment(st["number"], f"**Pull request is ready:** {pr['html_url']}\n\n"
                       "Review comments on the pull request and new comments here start another round.")
    old = st.get("pr_seen") or {}
    st["pr_seen"] = {"issue": max(cid, old.get("issue", 0)), "conv": 0, "inline": 0, "review": 0,
                     "updated_at": None, "issue_updated_at": st.get("seen_updated_at")}
    set_stage(st, "pr_open")


def collect_feedback(st, pr):
    """New review feedback from trusted people on the pull request and the issue, as prompt text parts."""
    n = st["pr_number"]
    seen = st["pr_seen"]
    trusted = CFG["initiators"]
    parts = []
    if pr["updated_at"] != seen.get("updated_at"):
        for r in gh("GET", repo_path(f"/pulls/{n}/reviews"), params={"per_page": 100}):
            body = (r.get("body") or "").strip()
            if (r["id"] > seen["review"] and r["user"]["login"] in trusted and body and MARK not in body
                    and r["state"] in ("CHANGES_REQUESTED", "COMMENTED")):
                parts.append(f"Review by {r['user']['login']} ({r['state']}):\n{body}")
            seen["review"] = max(seen["review"], r["id"])
        for c in gh("GET", repo_path(f"/pulls/{n}/comments"), params={"per_page": 100}):
            body = c.get("body") or ""
            if c["id"] > seen["inline"] and c["user"]["login"] in trusted and MARK not in body:
                hunk = "\n".join((c.get("diff_hunk") or "").splitlines()[-8:])
                line = c.get("line") or c.get("original_line") or "?"
                parts.append(f"Inline comment by {c['user']['login']} on {c['path']}:{line}:\n{body}\n\n"
                             f"Code context:\n```\n{hunk}\n```")
            seen["inline"] = max(seen["inline"], c["id"])
        for c in human_comments(n, after_id=seen["conv"]):
            parts.append(f"Pull request comment by {c['user']['login']}:\n{c['body']}")
            seen["conv"] = max(seen["conv"], c["id"])
        seen["updated_at"] = pr["updated_at"]
    if seen.get("issue_updated_at") != st.get("seen_updated_at"):
        for c in human_comments(st["number"], after_id=seen["issue"]):
            parts.append(f"Issue comment by {c['user']['login']}:\n{c['body']}")
            seen["issue"] = max(seen["issue"], c["id"])
        seen["issue_updated_at"] = st.get("seen_updated_at")
    save_state(st)
    return parts


def start_feedback_round(st, parts):
    log(f"#{st['number']}: {len(parts)} feedback item(s)")
    st["rounds"] = {"plan_review": 0, "impl_review": 0, "ci": 0}
    st["stage_started"] = None
    set_stage(st, "implement",
              "Reviewers left feedback on your work for this issue. Address every point: change the code where "
              "needed; where you disagree or the point is a question, answer it in `summary` "
              "(the summary is posted to the pull request).\n\n" + "\n\n---\n\n".join(parts))


def stage_pr_open(st):
    """Watch the open pull request. True when feedback started a new round or the PR was closed."""
    pr = gh("GET", repo_path(f"/pulls/{st['pr_number']}"))
    if pr["state"] != "open":
        set_stage(st, "done")
        return True
    parts = collect_feedback(st, pr)
    if not parts:
        return False
    start_feedback_round(st, parts)
    return True


def followup(st):
    """A finished issue got new comments: start another round. True when a round started."""
    pr = gh("GET", repo_path(f"/pulls/{st['pr_number']}"))
    parts = collect_feedback(st, pr)
    if not parts:
        return False
    st["followup"] = st.get("followup", 0) + 1
    if pr["state"] != "open":
        # The old branch is merged or abandoned: continue on a fresh one from the base branch, with a new PR.
        ensure_repo()
        wt = worktree(st)
        if wt.exists():
            git("worktree", "remove", "--force", str(wt))
        st["branch"] = f"issue-{st['number']}-followup-{st['followup']}"
        git("worktree", "add", "-B", st["branch"], str(wt), "origin/" + CFG["base_branch"])
        (wt / ".stamibot").mkdir(exist_ok=True)
        (wt / ".stamibot" / "plan.md").write_text(
            f"Follow-up round after pull request #{st['pr_number']} ({pr['html_url']}) was closed.\n"
            "There is no separate plan: the task is the reviewer feedback given in the prompt.\n", encoding="utf-8")
        st["pr_number"] = None
        st["pr_url"] = None
    start_feedback_round(st, parts)
    return True


STAGES = {"plan": stage_plan, "plan_review": stage_plan_review, "implement": stage_implement,
          "impl_review": stage_impl_review, "ci_push": stage_ci_push, "e2e": stage_e2e, "pr": stage_pr}


def try_retry(st):
    """A blocked issue resumes when an initiator comments `/retry [instructions]`."""
    for c in human_comments(st["number"], after_id=st.get("blocked_at_comment", 0)):
        body = (c.get("body") or "").strip()
        if body.startswith("/retry"):
            st["errors"] = 0
            st["rounds"] = {"plan_review": 0, "impl_review": 0, "ci": 0}
            if st.get("pr_seen"):
                st["pr_seen"]["issue"] = max(st["pr_seen"]["issue"], c["id"])
            note = body[len("/retry"):].strip()
            stage = st.get("blocked_from") or "plan"
            touched = st.pop("protected_block", None)
            if touched and note == "allow-protected":
                st["protected_ok"], note = head_sha(st), None  # the approval covers this commit only
            elif touched:
                stage = "implement"
                note = ("People did not approve your changes to protected paths. Undo every change to: "
                        + ", ".join(touched) + ".\n\n" + note).strip()
            set_stage(st, stage, note or None)
            return True
    return False


def advance(st):
    """Run stages until the issue waits for a person, CI, budget, or is finished."""
    while True:
        if st["stage"] == "blocked" and not try_retry(st):
            return
        if st["stage"] in TERMINAL:
            return
        if st.get("waiting") and not take_answers(st):
            return
        try:
            if st["stage"] == "ci_wait":
                if not stage_ci_wait(st):
                    return
            elif st["stage"] == "pr_open":
                if not stage_pr_open(st):
                    return
            else:
                STAGES[st["stage"]](st)
            st["errors"] = 0
            if st.get("paused"):
                st["paused"] = None
            save_state(st)
        except Paused as p:
            if st.get("paused") != str(p):
                st["paused"] = str(p)
                save_state(st)
                post_comment(st["number"], f"**Stamibot paused:** {p}. Work resumes automatically.")
            log(f"#{st['number']}: paused: {p}")
            return
        except Exception as e:  # a stage failure must not stop the other issues
            st["errors"] = st.get("errors", 0) + 1
            save_state(st)
            log(f"#{st['number']}: error {st['errors']}: {e}\n{traceback.format_exc()}")
            if st["errors"] >= 3:
                block(st, f"Repeated failure:\n\n```\n{str(e)[-3000:]}\n```")
            return


# ---------------------------------------------------------------- polling


def new_state(issue, stage, initiator=None):
    return {"number": issue["number"], "stage": stage, "initiator": initiator, "feedback": None,
            "waiting": None, "sessions": {}, "rounds": {"plan_review": 0, "impl_review": 0, "ci": 0},
            "errors": 0, "seen_updated_at": None}


def tick():
    issues = gh("GET", repo_path("/issues"), params={"assignee": CFG["assignee"], "state": "open",
                                                     "per_page": 100, "sort": "created", "direction": "asc"})
    issues = [i for i in issues if "pull_request" not in i]
    open_numbers = set()
    for issue in issues:
        n = issue["number"]
        open_numbers.add(n)
        st = load_state(n)
        changed = st is None or st.get("seen_updated_at") != issue["updated_at"]
        if st is None or (st["stage"] == "ignored" and changed):
            who = assigned_by(n)
            if who not in CFG["initiators"]:
                st = new_state(issue, "ignored")
                st["seen_updated_at"] = issue["updated_at"]
                save_state(st)
                log(f"#{n}: ignored, assigned by {who}")
                continue
            st = new_state(issue, "plan", who)
            st["issue"] = {"title": issue["title"], "body": issue.get("body"), "author": issue["user"]["login"]}
            save_state(st)
            log(f"#{n}: picked up, assigned by {who}")
            post_comment(n, f"**Stamibot picked this issue up** (assigned by @{who}).\n\n"
                         "Stages: plan, plan review, implementation, code review, CI, pull request. "
                         "Questions will be asked here.")
        st["issue"] = {"title": issue["title"], "body": issue.get("body"), "author": issue["user"]["login"]}
        # Waiting and blocked issues only move on a new comment: skip them while nothing changed.
        on_pr = (st.get("waiting") or {}).get("target") not in (None, n)
        if (st.get("waiting") or st["stage"] == "blocked") and not changed and not on_pr:
            continue
        st["seen_updated_at"] = issue["updated_at"]
        save_state(st)
        advance(st)
    for st in all_states():
        n = st["number"]
        if n in open_numbers:
            continue
        recent = time.time() - st.get("done_at", 0) < CFG["followup_days"] * 86400
        if st.get("followup") and st["stage"] not in ("done", "cancelled", "ignored"):
            # A follow-up round runs on a closed issue.
            st["seen_updated_at"] = gh("GET", repo_path(f"/issues/{n}"))["updated_at"]
            advance(st)
        elif st["stage"] == "done" and st.get("pr_number") and recent:
            st["seen_updated_at"] = gh("GET", repo_path(f"/issues/{n}"))["updated_at"]
            try:
                if followup(st):
                    advance(st)
            except Exception as e:
                log(f"#{n}: follow-up check failed: {e}")
        elif st["stage"] == "pr_open":
            set_stage(st, "done")  # the issue was closed, normally by merging the pull request
        elif st["stage"] not in TERMINAL:
            set_stage(st, "cancelled")  # closed or unassigned meanwhile


def status():
    for st in all_states():
        extra = "waiting for answer" if st.get("waiting") else (st.get("paused") or "")
        if st["stage"] == "pr_open":
            extra = "watching the pull request for feedback"
        print(f"#{st['number']:<5} {st['stage']:<12} {st.get('branch') or '-':<40} {st.get('pr_url') or ''} {extra}")
    limits, ts = read_budget()
    if limits:
        print(f"codex usage: {used_percent(limits):.0f}% (reading from {time.strftime('%Y-%m-%d %H:%M', time.localtime(ts))})")


def main():
    cmd = sys.argv[1] if len(sys.argv) > 1 else "tick"
    if cmd == "status":
        return status()
    if cmd != "tick":
        sys.exit("usage: bot.py [tick|status]")
    ROOT.mkdir(parents=True, exist_ok=True)
    lock = open(ROOT / "lock", "w")
    try:
        fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
    except OSError:
        return  # the previous tick is still working
    tick()


if __name__ == "__main__":
    main()
