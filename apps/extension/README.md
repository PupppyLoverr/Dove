# Dove for Chrome

A Manifest V3 extension that lets the agents in the Dove desktop app work in
your own browser (your logins, your tabs) on the sites you allow, while you
watch, stop or take over at any moment.

It is one of three ways agents browse in Dove:

| Path | What it is | When agents use it |
| --- | --- | --- |
| **ego lite** (`ego-browser`) | Your Chromium with an isolated Space per task | Default for web tasks when installed and healthy (the `dove-browser` skill) |
| **Dove's browser** (`browser_*` tools) | The in-app browser in the side panel | Local dev servers, or when you choose it in Settings › Plugins |
| **Dove for Chrome** (`chrome_*` tools) | This extension, in the browser you already use | Tasks that need your signed-in session in that browser |

The extension never takes over ego lite's Spaces, never opens windows on its
own except its review window, and acts only on tabs it is asked to.

## Supported browsers

Chromium browsers with Manifest V3 and the side panel API (Chrome 116+, Edge,
Brave). The end-to-end suite runs in whichever of Chrome, Edge and Brave is
installed; it has been run on Microsoft Edge 155 (Windows 11). Firefox and
Safari are not supported.

## What it can do

| Tool (in Dove: `chrome_<name>`) | |
| --- | --- |
| `tabs`, `tab_open`, `tab_close`, `tab_activate` | List and manage tabs |
| `navigate`, `wait` | Load pages; wait for text, a selector or the load (reports "still loading" instead of hanging) |
| `context` | A compact summary: title, URL, headings, forms and fields, dialogs, selection, counts |
| `snapshot` | An accessibility-style outline with refs (`[ref=e12]`), including open shadow roots and same-origin iframes |
| `find` | Elements by name and/or role, with refs |
| `extract` | Tables (rows), links, headings, form fields or a selector's text, as JSON |
| `read` | The page's text |
| `click`, `hover`, `type`, `press`, `scroll`, `select` | Act by ref; clicks go through the DevTools protocol as trusted input |
| `upload` | Attach files from the thread's folder to a file input |
| `screenshot` | PNG of the tab |
| `release` | Hand the browser back |

Results are compact text or JSON, capped at 200 000 characters. A ref that
went stale (the page navigated or re-rendered) fails with `STALE_REF` so the
agent takes a fresh snapshot instead of clicking the wrong thing.

## Trust model

- **Pairing.** Dove listens on `127.0.0.1` only. The extension's first message
  must carry the pairing token (64 hex characters from the OS CSPRNG), and
  Dove accepts only `chrome-extension://` origins. After the first pairing Dove
  remembers that extension and refuses any other until you choose **New
  pairing code** in Dove › Settings › Plugins.
- **Messages from Dove** are validated before anything runs: known message
  types, known tools, only that tool's arguments, plain-object arguments, at
  most 1 MB. Anything else gets an error reply.
- **Messages inside the browser.** Only the extension's own pages (side panel,
  review window) may pair, resume, allow or decide. Scripts injected into web
  pages may only ask to **Stop**, which is always safe.
- **Site access is opt-in per site.** The extension has no host permissions at
  install time (`optional_host_permissions` only). The first time an agent
  needs a site, a review window asks you; **Allow on this site** requests the
  browser permission for that origin from your click. Turning the site off in
  the side panel removes the permission.
- **Review before consequences.** Clicks that submit a form or whose control
  says things like *send, post, publish, buy, pay, order, delete, remove,
  transfer, merge, deploy, confirm*, pressing Enter in a form, typing with
  `submit`, and every upload wait for **Allow once** in the review window. A
  declined action fails with a message telling the agent not to retry. The
  review window is an extension page: a website cannot draw over it or click
  it.
- **Fields Dove never types into:** passwords, payment card fields, identity
  and bank numbers, one-time codes. The agent is told to ask you to fill them.
- **Untrusted content.** Text read from a page is returned inside
  `<page origin="…">…</page>` with a note that it is untrusted data, so an
  agent does not mistake "ignore your instructions" on a page for an
  instruction.
- **Stop.** The side panel's Stop and the in-page "Dove is controlling this
  tab" pill pause the extension, deny every pending review, and fail in-flight
  calls within ~100 ms (measured: 35 ms). Nothing runs until you Resume.
- **Uploads** are checked by Dove first: only existing files inside the
  asking thread's folder.
- **The activity log** keeps short summaries, never page contents or full
  payloads, in session storage (cleared when the browser closes).

## Develop

Load it unpacked: open `chrome://extensions` (or `edge://extensions`), turn on
Developer mode, **Load unpacked**, pick `apps/extension`. Click the Dove icon to
open the side panel, then paste the pairing code from Dove › Settings ›
Integrations › Dove for Chrome.

```sh
cd apps/extension
node --test "test/*.test.mjs"     # unit tests: policy, validation, reviews
node test/e2e/run.mjs             # browser evaluations in headless Chromium
```

`test/e2e/run.mjs` launches each installed Chromium browser headless with a
temporary profile and a copy of the extension, serves local fixture sites from
`127.0.0.1`, and drives the extension over the same wire protocol as Dove's
bridge (a test double in `test/e2e/harness.mjs`). The only change to the copy:
host access to the fixture origin is pre-granted in its manifest, because a
headless browser cannot show the permission prompt. Reviews are answered by
clicking the real review window's buttons. Results go to
`test/e2e/report.json`. Set `DOVE_E2E_BROWSERS` to a list of browser paths to
choose browsers.

## Release checklist

1. Bump `version` in `manifest.json`.
2. `node --test "test/*.test.mjs"` and `node test/e2e/run.mjs` pass in Chrome
   and Edge.
3. Pair with a release build of Dove and run QA prompts H, I and J from
   `docs/QA_PROMPTS.md` by hand.
4. Check `manifest.json` still has no `host_permissions` and only the
   permissions listed below.
5. Zip the folder without `test/`:
   `cd apps/extension && zip -r ../../dove-for-chrome-<version>.zip . -x 'test/*'`
6. Upload to the Chrome Web Store and Edge Add-ons; describe each permission:
   - `tabs`: list tabs and their titles/URLs for `chrome_tabs`.
   - `scripting`: run Dove's page functions in tabs you allowed.
   - `debugger`: trusted clicks, typing and screenshots (Chrome shows its
     "started debugging" bar while Dove acts).
   - `storage`: pairing, allowed sites, the activity log.
   - `sidePanel`, `alarms`: the side panel; reconnecting to Dove.
   - optional `http://*/*`, `https://*/*`: requested per site, from your click.

## Limits

- Cross-origin iframes are listed but not read or acted in.
- Closed shadow roots are invisible (by design of the web platform).
- Chrome shows a "Dove started debugging this browser" bar while an agent
  clicks or types; that is the price of trusted input events.
- The heuristics for consequential actions are word lists and form context.
  They catch common cases (the e2e suite covers order, delete and submit
  buttons); a site that labels a destructive button oddly may not be caught,
  which is why the side panel shows every action and Stop is always one
  click away.
