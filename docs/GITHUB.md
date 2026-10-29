# GitHub repository settings

Copy these into the repository's settings and pinned issue.

## About (description, ≤ 350 characters)

> Keplorayx is the agentic development environment: a native Rust desktop app that runs Claude Code, Codex, Cursor, Gemini CLI, OpenCode, Devin and more side by side, each in its own git worktree, with diff review, an agent-driven browser, generative UI and an iOS/Android remote. Local-first, free with your own keys.

(311 characters.)

Website: the Cloudflare Pages URL for `apps/landing` (see `apps/landing/README.md`), or the custom domain once one is set.

## Topics

```text
ai-agents, coding-agent, agentic-development, ade, developer-tools, rust, gpui,
desktop-app, claude-code, codex, cursor, gemini-cli, opencode, mcp,
agent-client-protocol, git-worktree, local-first, crdt, cloudflare-workers, open-source
```

(GitHub allows 20 topics; this is exactly 20.)

## Social preview

Upload a 1280×640 image: the Keplorayx mark and wordmark on the left, the main window screenshot (`apps/landing/assets/shots/hero.jpg`) cropped on the right, monochrome background matching the site.

Text on the image:

- Title: **Keplorayx**
- Line: **The agentic development environment.**
- Small print: **Many agents · one window · local-first**

## Suggested pinned issue

**Title:** Roadmap and how to get involved

**Body:**

> Keplorayx runs every coding agent you use in one native window. This issue tracks what's shipping next and where help is most useful.
>
> **Now**
> - Remote control: desktop ⇄ Cloudflare edge ⇄ iOS/Android with Woozlit sign-in
> - Office documents in the side panel (PDF, PPTX, XLSX, DOCX)
> - Integrations: Paper, Composio, computer use via cua-driver, Chrome extension browser control
>
> **Next**
> - Signed releases for macOS, Windows and Linux
> - Kanban and automations polish
> - Templates gallery
>
> **Good first contributions**
> - Add or improve an ACP agent adapter (`crates/dove-harness/src/acp.rs`)
> - Rendering fixes in the transcript (`apps/dove/src/transcript.rs`)
> - Linux packaging (Flatpak, AppImage, AUR)
>
> Read [CONTRIBUTING.md](../CONTRIBUTING.md), comment here with what you'd like to pick up, and we'll help you get started. Questions go to Discussions.

## Repository settings checklist

- Enable Discussions (Q&A, Ideas, Show and tell).
- Issue templates: bug report (OS, commit, agent CLI + version, logs) and feature request.
- Labels: `good first issue`, `help wanted`, `agent:<name>`, `platform:macos|windows|linux`, `area:ui|engine|harness|edge|mobile`.
- Branch protection on `main`: require the CI workflow.
