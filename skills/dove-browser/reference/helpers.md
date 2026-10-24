# ego-browser API quick reference

Written against ego-browser 2.1.0. The installed version's guide
(`ego-browser skill`) and `help("<name>")` are the source of truth: when
they disagree with this page, they win. ego-browser is not Playwright; use
only what is listed here or in `help()`.

All times are milliseconds. Every `ego-browser nodejs` call is a new Node
process: spaces, tabs and page labels persist between rounds, variables do
not.

## Spaces

| Call | Use |
|---|---|
| `await taskSpace("3-6 word name")` | Create the one space for this goal, or resume your own space of that name |
| `await taskSpace(<id>)` | Resume by the numeric `spaceId` printed in round 1 |
| `await listTaskSpaces()` | Entries expose `id`, `name`, `ownership` (`"user"` = the user's) |
| `await claimTaskSpace(<id>)` | Take a user-owned space, only when the user asks |
| `await takeOverTaskSpace(<id>)` | Resume after `handOff()` once the user says go |
| `task.spaceId`, `task.name`, `task.ownership` | State |
| `task.page("p1")` | A page that already exists (never creates; always truthy) |
| `await task.newPage()` | Another blank page when several must stay open |
| `await task.pages()` / `await task.tabs()` | Managed pages / every tab (`label?`, `page`, `url`, `active`, `openedBy`) |
| `await task.adopt(page)` | Manage an unlabelled tab before using it |
| `task.release(label)` | Hand an unknown-origin page back without closing it |
| `task.userPage()` | The page the user was on, after a hand-off |
| `await task.handOff()` | Give control to the user (sign-in, CAPTCHA, 2FA, payment) and end the round |
| `await task.waitForControl(opts)` | Wait in place for control to come back |
| `await task.finish({ keep: [] })` | Done: exactly once, last; `keep: ["p2"]` leaves a result page open |
| `task.cdp(method, params)` | Target/Browser-level CDP |

## Page: state and observation

`page.label`, `page.url()`, `page.title()`, `page.info()` (dialogs, state),
`page.snapshot()` (viewport), `page.snapshot({ scope: "full_page" })`,
`page.snapshot({ scope: "subtree", root: "@12" })` (inside an iframe),
`page.screenshot({ path: "C:/abs/path.png" })` (absolute path, forward
slashes; omit for a temp file).

## Page: navigation and waits

`goto(url)`, `reload()`, `waitForURL(urlOrRegex)`,
`waitForSelector(sel, { state: "visible" | "hidden", timeout })`,
`waitForLoadState()` (default `load`),
`waitForFunction(fn, undefined, { timeout })` (Playwright argument order),
`waitForEvent("popup" | "download")`, `waitForFileChooser({ timeout })`,
`waitForTimeout(ms)` (last resort).

## Page: elements

`click(sel, { label })`, `dblclick`, `hover`, `dragAndDrop(from, to)`,
`fill(sel, value)`, `selectOption(sel, value | { label } | { index } | [..])`,
`focus(sel)`, `press(sel, "Enter")`, `setInputFiles(sel, ["C:/abs/file"])`,
`close()`.

Only pointer actions take `label` (3-6 words, shown beside the agent
cursor): `click`, `dblclick`, `hover`, `dragAndDrop`, `mouse.click`,
`mouse.move`, `mouse.wheel`. In `selectOption(sel, { label })`, `label` is
the option's visible text instead.

## Selectors

| Form | Example |
|---|---|
| Snapshot ref | `@21`, `ref=21` (only valid against the latest snapshot) |
| Role locator | `loc=role:button[name='Save']`, `[name*='Sav']` for a substring |
| Text | `text=save` (any case, substring), `text="Save changes"` (exact) |
| CSS | `css=form button.primary`, raw `#id` / `.class`, `:has-text("x")`, `>> nth=0` |
| Href | `loc=href:/settings` |
| XPath | `xpath=//button[1]` |

Actions need exactly one match; they search the top document first, then
frames. Actions scroll their target into view themselves.

## Pointer and keyboard (visual pages)

`page.mouse.click(x, y, { label })`, `move`, `down`, `up`,
`wheel(dx, dy, { label })` (move over the area first);
`page.keyboard.press("ControlOrMeta+A")`, `type(text)`, `insertText(text)`,
`paste(text | { text, html })`, `down`, `up`. Coordinates are CSS pixels.

## Dialogs, popups, files, requests

- Dialogs: `receipt.dialog` or `page.info()`, then `page.acceptDialog(text?)`
  or `page.dismissDialog()`.
- Popups: arm `page.waitForEvent("popup")` before the click, or read
  `receipt.popups[0].label`.
- Downloads: arm `page.waitForEvent("download")`, click, then
  `await download.saveAs("C:/abs/file")` in the same round.
- Uploads: `page.setInputFiles(sel, [path])`, or arm `waitForFileChooser()`
  and `chooser.setFiles(path)`.
- In-page requests: `page.fetch(url, { method, headers, body, timeout })`
  returns `{ ok, status, url, headers, body }`; `{ saveAs }` for binaries.

## Escape hatches

- `page.evaluate(fn, arg)`: runs in the page; one JSON-serialisable
  argument; returns JSON. No timeout option: keep it bounded.
- `page.cdp(method, params)`: Page/Runtime/DOM/Network/Input. Invalidates
  refs.
- `await page.events()`: returns and clears buffered protocol events.

## Shell forms

- bash/zsh: `ego-browser nodejs <<'EOF' … EOF`
- PowerShell: write `round-NN.mjs` with your file tool, then
  `$null | ego-browser nodejs -e "await import('file:///C:/abs/round-NN.mjs')"`
  (closing stdin avoids a hang in 0.6.0.16).
- cmd: `ego-browser nodejs < C:\abs\round-NN.mjs`
