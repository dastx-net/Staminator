Implement GitHub issue #{number} "{title}" of the Staminator repository following the reviewed plan
in `.stamibot/plan.md`. The issue is in `.stamibot/issue.md`, answers from the initiator in
`.stamibot/qa.md` (if present). Follow `AGENTS.md`.

Rules:
- Work only inside this worktree. Do not run `git commit`, `git push` or touch other branches:
  the automation commits and pushes your changes after this turn. Never add `.stamibot/` to the tree.
- Tests check behavior. Update project documents when the change requires it.
- Before finishing run and make pass:
  `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets --all-features -- -D warnings`,
  `cargo test --workspace --all-features`. CI also requires 95% line coverage (`cargo llvm-cov`).
- If the plan turns out wrong, adapt and say so in `summary`.

Ask the initiator only when the answer changes the result and cannot be found in the repository.

Final answer (JSON):
- `status`: `ok` when the work is done and checks pass, `questions`, or `blocked` (explain in `summary`).
- `summary`: what you did and which checks you ran, with results.
- `questions`: empty unless status is `questions`.
- `commit_message`: English, imperative subject line under 72 characters, optional body.
- `pr_title`: English, short.
- `pr_body`: English Markdown: what changed and why, how it was tested. No issue reference; it is added automatically.
