# Troubleshooting

| Symptom | Cause | Do this |
|---|---|---|
| `Unknown ref` / `@N` not found | The page changed since the snapshot, or the element is outside the viewport snapshot | Take a new snapshot (`scope: "full_page"` if needed) and use the new ref, or a `loc=role:` selector |
| "matches N elements" | Selector actions need exactly one match | Narrow it: `loc=role:button[name='Save']`, quoted `text="Save"`, or `>> nth=0` |
| Click "worked" but nothing changed | The receipt only means it was dispatched | Look at the page: overlay or modal in the way? disabled button? validation error? Fix the cause, don't repeat the click |
| Navigation race (snapshot shows the old page) | Waited for nothing | Arm `waitForURL` / `waitForSelector` for the expected state in the same round, then snapshot |
| Popup or new tab "missing" | Wait started after the click | Start `page.waitForEvent("popup")` before the click, or read `receipt.popups` |
| A JavaScript `alert`/`confirm`/`prompt` blocks the page | Synchronous dialog | `page.acceptDialog()` / `page.dismissDialog()` (ask the user if it confirms something irreversible) |
| `ego-browser nodejs -e …` never returns | ego-browser 0.6.0.16 waits on an open stdin even with `-e`; harness shells keep it open | Close stdin: `$null \| ego-browser …` in PowerShell, `… < /dev/null` in bash |
| `SyntaxError: await is only valid…` | The shell mangled the script | Use the `.mjs` file + `import('file:///…')` form; do not wrap in an IIFE |
| Non-ASCII text arrives garbled on Windows | PowerShell 5.1 encoding | Keep the text inside a UTF-8 `.mjs` written with your file tool; never in `-e` or piped via `Get-Content` |
| Paths fail on Windows | Backslashes are escapes in JS strings | Use forward slashes: `C:/Users/me/file.pdf` |
| `process.env.X` is undefined | Scripts don't inherit your shell env | Write values into the script |
| `EGO_TASK_SPACE_NAME_IN_USE` | The user owns a space with that name | Ask whether to work in it (`claimTaskSpace(id)`) or pick another name |
| The page stops responding / timed out | Heavy page or long `evaluate` | Keep `evaluate` bounded; check `executionStopped` / `mayHaveLateEffects`; reload the page in the same space. If still stuck, stop and ask; never open a new space to escape |
| The user took control | They are using the space | Stop. Do not retry or route around it. Resume only after they say so |
| `[ego-browser:notice]` in output | An ego lite update exists | Finish the task, tell the user, run `ego-browser upgrade` only with their OK, then `ego-browser skill` again |
| Site blocks automation / CAPTCHA | Bot detection | Hand off to the user; never try to solve or bypass it |
| `ego-browser` not found after install | PATH only reaches new processes on Windows | Ask the user to restart the agent (or Dove) after finishing ego lite onboarding |
