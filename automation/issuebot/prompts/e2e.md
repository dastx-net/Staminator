End-to-end check for GitHub issue #{number} "{title}" of the Staminator repository.
CI passed. Now run the Windows build in the test VM and verify the change with your own eyes.
Do not change project files in this turn.

Context: `.stamibot/issue.md` (the issue), `.stamibot/plan.md` (the plan), and the implementer's summary:

{summary}

Windows build to test: `{exe}`

Tool for the VM (screen and input over the VM console, files and commands over SSH): `{vm}`
- `{vm} put {exe} C:/e2e/staminator-desktop.exe` - copy the build (`put` also copies folders)
- `{vm} run C:/e2e/staminator-desktop.exe` - start it on the visible desktop
- `{vm} shot .stamibot/screen.png` - screenshot (1280x800); then look at the PNG with your image viewer tool
- `{vm} click X Y [--right] [--double]`, `{vm} type TEXT`, `{vm} key enter|esc|alt-f4|ctrl-s|...`
- `{vm} ps 'PowerShell command'` - run a command in the VM (no desktop access from there)
- `{vm} kill staminator-desktop` - stop the program

Rules:
- Touch only the Staminator windows. The VM is shared: do not open, read or click Microsoft Teams or any
  other application, and do not change Windows settings.
- Always take a fresh screenshot after an action before deciding the next one; coordinates come from the
  latest screenshot only.
- Keep it short: at most about 25 actions. Verify what this issue changed. If the change has no visible
  effect in the UI (docs, refactoring, CI), do only a smoke test: the main window opens, a menu opens, the
  program closes.
- Known limitation, not a failure: the build may show "no text for the selected language" because lesson
  data is looked up at a build-machine path.
- When finished, stop the program with the `kill` command.

Final answer (JSON):
- `status`: `pass`, `fail` (the change does not work or the program is broken), or `skipped` (you could
  not test for reasons unrelated to the change; explain).
- `summary`: what you did and saw, two to five sentences.
- `problems`: concrete defects for the implementer; empty unless status is `fail`.
