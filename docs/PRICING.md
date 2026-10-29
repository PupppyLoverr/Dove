# Keplorayx pricing

Keplorayx is free to download and free to use with your own agents and keys. Paid plans cover what costs real money to run or to support: hosted sync for phones and multiple machines, team features, and enterprise controls.

## Principles

1. **BYOK stays free, forever.** Keplorayx never resells model tokens. You pay Anthropic, OpenAI, Google and others directly, or use the subscriptions you already have (Claude, ChatGPT, Cursor, Copilot). No markup, no credits.
2. **Everything local is free.** Any feature that runs on your machine (all agents, worktrees, diffs, browser control, generative UI, office documents, integrations, usage ledger) is in the free plan.
3. **Pay for hosted, shared and governed.** Hosted relay capacity, history retention, team spaces, SSO and audit are the paid surface.
4. **The relay is open source.** Anyone can self-host the Cloudflare Worker on their own free-tier account and get remote control without paying Keplorayx.

## Plans

| | **Free** | **Pro** | **Team** | **Enterprise** |
| --- | --- | --- | --- | --- |
| Price | $0 | $12/mo, or $10/mo billed yearly | $24/user/mo, or $20/user/mo billed yearly | Custom, annual |
| Agents, worktrees, diffs, terminal | Unlimited | Unlimited | Unlimited | Unlimited |
| Browser control (in-app + Chrome extension) | ✓ | ✓ | ✓ | ✓ |
| Generative UI, office docs, Paper, Composio (own key), computer use | ✓ | ✓ | ✓ | ✓ |
| Usage ledger across agents | ✓ | ✓ | ✓ | Org-wide reporting |
| Mobile remote via Keplorayx's hosted relay | 1 desktop + 1 phone | Unlimited devices | Unlimited devices | Unlimited devices |
| Remote session history on the relay | 24 hours | 90 days | 1 year | Custom retention |
| Push notifications (needs you / done / failed) | — | ✓ | ✓ | ✓ |
| Start new runs from the phone | ✓ | ✓ | ✓ | ✓ |
| Sync settings, templates and automations across machines | — | ✓ | ✓ | ✓ |
| Shared projects, threads and templates | — | — | ✓ | ✓ |
| Hand off a running thread to a teammate | — | — | ✓ | ✓ |
| Central billing, seat management | — | — | ✓ | ✓ |
| SSO (SAML/OIDC), SCIM | — | — | — | ✓ |
| Audit log export, policy (allowed agents/models, permission modes) | — | — | — | ✓ |
| Self-hosted relay with support, data residency | Self-host without support | Self-host without support | Self-host without support | ✓ with support |
| Support | Community | Email | Priority email | Dedicated, SLA |

Students and maintainers of open-source projects with real users get Pro for free on request.

## Why these numbers

Benchmarks below come from each vendor's public pricing page when this plan was written (October 2026). Re-check them before any launch copy quotes them, because these pages change often.

| Product | Individual | Team | What you are paying for |
| --- | --- | --- | --- |
| Cursor | Free; Pro ~$20/mo; higher tiers up to ~$200/mo | Teams ~$40/user/mo | Editor plus bundled model usage |
| Warp | Free; paid individual ~$18–20/mo | Business tier per seat, above individual | Terminal plus bundled AI requests |
| Conductor | Free with your own Claude/Codex plan; paid tiers ~$50–60/mo observed | Teams and Enterprise listed | Mac app orchestrating Claude Code/Codex worktrees |
| Codex | Included with ChatGPT Plus ($20/mo) and Pro ($200/mo) | Business/Enterprise per seat | Model access plus the agent |
| Zeron | Open source desktop (MIT); hosted features are the paid surface | — | Agent workspace, the design reference for Keplorayx |

What follows from this:

- Cursor, Warp and Codex charge $18–20 at the entry tier mostly because **they bundle model inference**. Keplorayx bundles none, so its Pro price has to sit clearly below that line. $12 ($10 yearly) is close to the price of one hosted service, which is what Pro is.
- Conductor shows that people pay for orchestration alone, but its paid tiers are above what a sync add-on can justify for individuals.
- Team at $24/user is below Cursor Teams (~$40) and comparable to Warp's team tier. It is still twice Pro, because shared state and seat management carry most of the cost.
- Enterprise is custom because SSO, audit and self-hosted relay support are sales-led everywhere in this market.

## Unit economics (hosted relay)

The relay is a Cloudflare Worker with one Durable Object per session plus a per-account registry. Text-only session traffic is small (transcript deltas and command frames). Cloudflare's free tier covers individual Free users at launch volume. Paid Workers usage-based pricing applies beyond that, so Free limits are set on devices and retention, which are what drive storage and request volume. Gross margin on Pro and Team should stay above 90%.

## Gating rules for engineers

- Gate on the **relay**, never in the desktop binary. A self-hosted relay has no limits.
- A gated feature must explain itself in the UI ("Push notifications are part of Pro") and link to pricing. It must never break a free user's local work.
- No trial that silently converts. Pro has a 14-day trial with no card required.
