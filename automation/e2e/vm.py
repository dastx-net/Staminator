#!/usr/bin/env python3
"""Eyes and hands for end-to-end checks in the Windows test VM.

Screen and input go through the VM's VNC console (hypervisor level, nothing installed in the guest);
files and commands go through SSH. See automation/README.md.

  vm.py status                 is the VM reachable, is the test user logged in, is RDP in use
  vm.py console                put the test user's desktop on the VNC console (drops an RDP connection)
  vm.py shot [FILE]            screenshot to FILE (default ./.stamibot/screen.png); prints the path
  vm.py click X Y [--right] [--double]
  vm.py move X Y
  vm.py type TEXT              ASCII text, typed key by key
  vm.py key COMBO...           e.g. enter, esc, alt-f4, ctrl-s, super-r
  vm.py ps COMMAND             run a PowerShell command over SSH (no desktop: session 0)
  vm.py put LOCAL REMOTE       copy a file or a folder into the VM (REMOTE like C:/e2e/app.exe)
  vm.py run EXE [ARGS]         start a program on the visible desktop
  vm.py kill NAME              stop processes by image name (e.g. staminator-desktop)
"""

import os
import socket
import subprocess
import sys
from pathlib import Path

VNC = os.environ.get("VM_VNC", "127.0.0.1::5900")
SSH_HOST = os.environ.get("VM_SSH", "igor@192.168.123.60")
SSH_KEY = os.environ.get("VM_KEY", str(Path.home() / ".ssh" / "win11_vm"))
VNCDO = os.environ.get("VNCDO", str(Path.home() / "stamibot" / "venv" / "bin" / "vncdo"))
SSH_OPTS = ["-i", SSH_KEY, "-o", "BatchMode=yes", "-o", "ConnectTimeout=10", "-o", "LogLevel=ERROR",
            "-o", "StrictHostKeyChecking=accept-new"]
# Tests run on the desktop of a separate unprivileged Windows user; SSH_HOST is the administrator who controls it.
DESKTOP_USER = os.environ.get("VM_USER", "e2e")
TASK = "stamibot-e2e"


def vnc(*args):
    subprocess.run([VNCDO, "-s", VNC, *map(str, args)], check=True, timeout=60)


def ps(command, check=True):
    """Run PowerShell in the VM. The SSH default shell there is PowerShell."""
    # Windows console tools answer in the OEM code page: never fail on decoding.
    res = subprocess.run(["ssh", *SSH_OPTS, SSH_HOST, command], capture_output=True, timeout=180,
                         encoding="utf-8", errors="replace")
    if check and res.returncode != 0:
        sys.exit(f"vm: command failed ({res.returncode}): {res.stderr.strip() or res.stdout.strip()}")
    return res.stdout.strip()


def reachable(host, port):
    try:
        with socket.create_connection((host, port), timeout=3):
            return True
    except OSError:
        return False


def status():
    """Dict: ssh, vnc, session (id of the test user's desktop or None), console (id of the session shown
    on the VNC console), rdp (an RDP connection is live)."""
    host = SSH_HOST.split("@")[-1]
    vnc_host, vnc_port = VNC.split("::")
    info = {"ssh": reachable(host, 22), "vnc": reachable(vnc_host, int(vnc_port)), "session": None,
            "console": None, "rdp": False}
    if info["ssh"]:
        out = ps("$p = Get-Process explorer -IncludeUserName -ErrorAction SilentlyContinue | "
                 f"Where-Object {{ $_.UserName -like '*\\{DESKTOP_USER}' }} | Select-Object -First 1; "
                 "if ($p) { 'session=' + $p.SessionId }; if (qwinsta | Select-String 'rdp-tcp#') { 'rdp=1' }; "
                 "$c = qwinsta | Select-String '^.console\\s+(\\S+\\s+)?(\\d+)'; "
                 "if ($c) { 'console=' + $c.Matches[0].Groups[2].Value }",
                 check=False)
        for line in out.splitlines():
            if line.startswith("session="):
                info["session"] = int(line.split("=")[1])
            if line.startswith("console="):
                info["console"] = int(line.split("=")[1])
            if line.strip() == "rdp=1":
                info["rdp"] = True
    return info


def main():
    args = sys.argv[1:]
    if not args or args[0] in ("-h", "--help"):
        print(__doc__)
        return
    cmd, rest = args[0], args[1:]
    if cmd == "status":
        print(" ".join(f"{k}={v}" for k, v in status().items()))
    elif cmd == "console":
        info = status()
        if info["session"] is None:
            sys.exit(f"vm: user {DESKTOP_USER} is not logged in to the VM (it logs in automatically at boot)")
        if info["console"] != info["session"]:
            # Only SYSTEM may attach another user's session without its password.
            ps(f"$a = New-ScheduledTaskAction -Execute 'tscon.exe' -Argument '{info['session']} /dest:console'; "
               "Register-ScheduledTask -TaskName stamibot-console -Action $a -User 'SYSTEM' -RunLevel Highest "
               "-Force | Out-Null; Start-ScheduledTask -TaskName stamibot-console; Start-Sleep -Seconds 4")
            if status()["console"] != info["session"]:
                sys.exit("vm: could not move the test desktop to the console")
        vnc("move", 640, 400)  # wake the display
        print(f"desktop of {DESKTOP_USER} is on the console")
    elif cmd == "shot":
        target = Path(rest[0] if rest else ".stamibot/screen.png")
        target.parent.mkdir(parents=True, exist_ok=True)
        vnc("capture", target)
        print(target)
    elif cmd == "click":
        x, y = rest[0], rest[1]
        button = 3 if "--right" in rest else 1
        clicks = ["click", button] * (2 if "--double" in rest else 1)
        vnc("move", x, y, *clicks)
    elif cmd == "move":
        vnc("move", rest[0], rest[1])
    elif cmd == "type":
        vnc("type", " ".join(rest))
    elif cmd == "key":
        for combo in rest:
            vnc("key", combo)
    elif cmd == "ps":
        print(ps(" ".join(rest)))
    elif cmd == "put":
        remote = rest[1].replace("\\", "/")
        ps(f"New-Item -ItemType Directory -Force -Path (Split-Path -Parent '{remote}') | Out-Null")
        if os.path.isdir(rest[0]):  # scp would nest the folder inside an existing one
            ps(f"Remove-Item -Recurse -Force -ErrorAction SilentlyContinue '{remote}'")
        subprocess.run(["scp", "-q", "-r", *SSH_OPTS, rest[0], f"{SSH_HOST}:{remote}"], check=True, timeout=900)
        print(remote)
    elif cmd == "run":
        # SSH sessions have no desktop: a scheduled task marked "interactive" starts the program on it.
        exe, exe_args = rest[0].replace("/", "\\"), " ".join(rest[1:])
        user = DESKTOP_USER
        folder = exe.rsplit("\\", 1)[0]
        action = (f"New-ScheduledTaskAction -Execute '{exe}' -WorkingDirectory '{folder}'"
                  + (f" -Argument '{exe_args}'" if exe_args else ""))
        ps(f"$a = {action}; $p = New-ScheduledTaskPrincipal -UserId '{user}' -LogonType Interactive; "
           f"Register-ScheduledTask -TaskName {TASK} -Action $a -Principal $p -Force | Out-Null; "
           f"Start-ScheduledTask -TaskName {TASK}")
        print("started " + exe)
    elif cmd == "kill":
        ps(f"Stop-Process -Name '{rest[0]}' -Force -ErrorAction SilentlyContinue")
    else:
        sys.exit(f"vm: unknown command {cmd}\n{__doc__}")


if __name__ == "__main__":
    main()
