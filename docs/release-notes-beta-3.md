# Keplorayx 0.1.0 beta 3

Keplorayx is the new name of this app. Earlier builds were called Dove. The program, the installer, and the project on disk still use the dove code name.

## What changed

- Signing in with Woozlit pulls the sessions already on the account from the Cloudflare edge and shows them here. A session that lives on another device is not claimed by this computer, and its transcript is filled in when this computer does not already have one. Sessions on this computer are published to that same account, which is what the phone reads.
- The window asks once, and only when no account is stored, whether to stay on this computer or log in with Woozlit. Staying local does not pair a phone or another computer. Those need the Woozlit login, and they talk to `https://cosmos-edge.cosmos-edge.workers.dev`.
- Choosing Cloud on the machine chip keeps that thread on Cloud. Sending does not start an agent on this computer. The line under the composer is the same one Zeron uses while a machine is starting or asleep.
- Settings → Machines can connect GitHub through the cosmos-ade GitHub App (device flow, then install). The token is saved on this computer and is not shown.
- A working session row shows a spinner. Nested sessions carry a type mark. Resting on a row opens a card with the session name, harness, folder, branch, and worktree.
- Moving wallpaper plays on Home. A chat keeps a still frame so a video is not decoded while the window sits idle.
- A bad GIF no longer has to be decoded on the UI thread just to decide that it moves.

## Cloud is not live yet

Cloud providers do not work until that provider has been connected and the connection has answered. New box stays off until then. A saved API key is not, by itself, a working cloud machine. InstaBox and the other boxes are not listed as ready.

## Known issues

- The app is not code-signed, and the macOS build is not notarized.
- Pairing needs the mesh host (`dove-mesh-host`) beside the app. A package built before that binary existed cannot pair.
