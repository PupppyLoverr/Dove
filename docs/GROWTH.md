# Growth plan: 100,000 users in 12 weeks

A **user** here is a person who installed Keplorayx and started at least one agent session (counted by the opt-in anonymous "first session" ping, or by Woozlit sign-in for mobile). Stars and downloads are tracked but are not the goal.

Target: **100,000 activated users by the end of week 12**, with ≥ 25% week-4 retention.

## Positioning

- One line: **Keplorayx is the agentic development environment. Every coding agent, one native window.**
- Who first: developers who already pay for two or more of Claude Code, Codex, Cursor and Copilot and juggle them in terminals.
- Wedge: run three agents on the same task in three worktrees, compare the diffs, merge the best one. Nobody else does this natively across vendors.
- Proof points to repeat everywhere: native Rust (cold start and memory), free with your own keys, open source (Apache-2.0), phone remote.

## Funnel and weekly targets

Funnel: **visit site/repo → download → first session (activation) → day-7 active → Pro**.

Planning ratios: download/visit 12%, activation/download 60%, D7/activation 35%, Pro/activated 2% by week 12.

| Week | Phase | New visitors | Downloads | New activated | Cumulative activated | GitHub stars (cum.) | Pro (cum.) |
| --- | --- | --- | --- | --- | --- | --- | --- |
| 1 | Private beta | 8k | 1.2k | 700 | 700 | 1.5k | — |
| 2 | Private beta | 12k | 1.8k | 1.1k | 1.8k | 3k | — |
| 3 | Launch week | 120k | 15k | 9k | 10.8k | 12k | 80 |
| 4 | Launch tail | 60k | 7.5k | 4.5k | 15.3k | 16k | 150 |
| 5 | Mobile launch | 70k | 9k | 5.5k | 20.8k | 19k | 250 |
| 6 | Templates | 80k | 10k | 6.2k | 27k | 22k | 380 |
| 7 | Integrations | 90k | 11.5k | 7k | 34k | 25k | 520 |
| 8 | Teams | 100k | 13k | 8k | 42k | 28k | 700 |
| 9 | Second wave | 130k | 17k | 10.5k | 52.5k | 32k | 950 |
| 10 | Creators | 140k | 19k | 11.5k | 64k | 36k | 1.2k |
| 11 | Referral peak | 160k | 22k | 13.5k | 77.5k | 40k | 1.5k |
| 12 | Launch 1.0 | 200k | 27k | 16.5k | 94k–100k+ | 45k | 1.9k |

If cumulative activation is more than 20% under plan at the end of week 5, pull the week-9 second wave forward to week 7 and double creator spend of time (not money) in weeks 6–8.

## Sequencing

### Weeks 1–2: private beta (seed 2,000 real users)

- Signed builds for macOS, Windows and Linux before anything public. A broken first launch wastes the launch.
- Invite from: the Zeron/Cosmos community, agent CLI Discords (Claude Code, OpenCode, goose, Gemini CLI), r/ClaudeAI and r/ChatGPTCoding power users, people who starred similar repos.
- Daily triage of the top crash and onboarding drop-off. Fix the first-session path until activation/download ≥ 60%.
- Collect 20 short clips and quotes for launch week. Ask permission to quote.

### Week 3: launch week

Order matters: each channel feeds the next.

1. **Tuesday, Show HN** at ~08:00 US Eastern. Title: "Show HN: Keplorayx – run Claude Code, Codex and Gemini side by side in worktrees (Rust, open source)". The founder answers every comment for 12 hours. Link the repo, not the landing page.
2. **Tuesday afternoon, X/Twitter thread** quoting HN traction: a 30-second video of three agents racing on one task, then the diff compare. Under 280 characters per post; the video carries the message.
3. **Wednesday, Reddit**: r/rust (the GPUI/native angle), r/ClaudeAI, r/ChatGPTCoding, r/LocalLLaMA (BYOK, local-first), each with a post written for that community.
4. **Thursday, Product Hunt** at 00:01 Pacific, with HN and X social proof in the first comment. Line up beta users who already have PH accounts; never ask for upvotes, ask people to try it and comment.
5. **Friday, newsletters and roundups**: pitch TLDR, Console.dev, This Week in Rust, Changelog, Ben's Bites with the video and download numbers.
6. Keep a public launch-week changelog with daily fixes. Shipping fast in public is the strongest second post.

### Week 5: mobile launch

iOS on TestFlight and then the App Store, Android on Play. Message: "approve your agent from your phone". A second X/HN moment ("Show HN: control Claude Code from your phone, open-source relay on Cloudflare").

### Weeks 6–8: templates, integrations, teams

- Templates gallery (below), one featured template a day on X.
- Integration launches co-marketed with partners: Paper, Composio, cua-driver. Each partner shares to their audience; ask for a retweet and a docs link.
- Team plan launch with three design-partner teams' stories.

### Weeks 9–12: second wave and 1.0

- Second HN post only with genuinely new substance (for example a benchmark: Keplorayx vs. Electron alternatives on memory and startup with a reproducible script).
- YouTube: send builds and talking points to 30 developer creators; one video from a large channel is worth a launch.
- 1.0 release in week 12 with a "100k developers" post.

## Open-source loops

- **Built-in attribution, off by default.** When an agent opens a PR from Keplorayx, offer an opt-in "Made with Keplorayx" line in the PR body. Every enabled PR is a backlink seen by reviewers.
- **Adapters as contributions.** Every new agent CLI is an ACP adapter away. Publish an "add your agent" guide; each agent vendor that merges an adapter announces it to their users.
- **Good first issues, always ten open.** Fast reviews (under 24 hours) turn first-time contributors into advocates.
- **Self-hostable relay** on Cloudflare's free tier with a one-click deploy button. Infrastructure people share deploy buttons.
- **Public roadmap issue** (pinned, see `docs/GITHUB.md`) and a weekly changelog.

## Referral

- Every user gets a link. When a referred user activates, both get **one month of Pro** (stackable up to 12 months).
- Teams: invite three teammates to a Team trial and the trial extends to 30 days.
- Show the referral offer once, after the user's third successful session, never in the first run.
- Target: 15% of new activations from referral by week 10.

## Templates

A template is a project starter with prompts, a worktree layout, agent choices and MCP tools pre-wired, shared as a small file or a URL that opens in Keplorayx.

- Launch with 25: "three agents race a bug", "Claude plans, Codex implements, Gemini reviews", "UI from a Paper design", "office report from a CSV", "fix failing CI", "upgrade a dependency across the monorepo", "write tests to 80% coverage".
- Every template page has its own URL on the site (SEO) and a "Open in Keplorayx" button (deep link that installs Keplorayx first if needed).
- Community submissions through PRs to a `templates/` repo; featured weekly.

## Metrics

Instrument from day one with an **opt-in**, anonymous, self-hosted pipeline (a Cloudflare Worker writing to D1/R2 on the free tier; no third-party analytics SaaS). Events: `install`, `first_session` (agent kind only), `session_day`, `worktree_created`, `pr_opened`, `mobile_paired`, `template_opened`, `referral_activated`, `plan_upgraded`. No prompts, code or file paths, ever.

Weekly dashboard:

| Metric | Healthy by week 12 |
| --- | --- |
| Activation (first session ÷ downloads) | ≥ 60% |
| D7 retention of activated | ≥ 35% |
| W4 retention of activated | ≥ 25% |
| Sessions per weekly active user | ≥ 15 |
| Users running ≥ 2 different agents | ≥ 40% of WAU |
| Mobile paired ÷ WAU | ≥ 20% |
| Referral share of new activations | ≥ 15% |
| Free → Pro conversion | ≥ 2% |
| Crash-free sessions | ≥ 99.5% |

## Risks

- **A broken launch build.** Mitigate with a week of private beta and auto-update before week 3.
- **Agent CLI breaking changes.** Pin known-good versions in the adapter tests; ship adapter fixes within 24 hours.
- **Vendor bundling** (an agent vendor shipping its own multi-agent UI). Keplorayx's answer is cross-vendor: the more vendors exist, the more Keplorayx is needed.
- **Over-counting.** Report activated users, not downloads or stars.
