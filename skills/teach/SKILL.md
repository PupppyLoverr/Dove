---
name: teach
description: Teach Dove a new reusable skill from the user's explanation, interview the user about a task, write it as an Agent Skill that every agent in Dove can use, then rehearse it on a fresh example before it is trusted. Use when the user says "teach Dove", "teach you how to", "remember how to do X", "turn this into a skill", "make a skill for", or asks to rehearse, test or publish a taught skill.
---

# Teach

You are helping the user teach Dove a task once so that every agent Dove runs (Claude, Codex,
OpenCode, Gemini, Cursor and the others) can do it afterwards. The result is a normal Agent Skill
folder: `SKILL.md`, `evals/`, optional `references/` and `scripts/`, plus `TEACH.json` (provenance
and rehearsal record, written only by the script).

Script: `scripts/teach_tool.py` (standard library only). Run it with `uv run scripts/teach_tool.py ...`
or `python3 scripts/teach_tool.py ...`. Paths below are relative to this skill folder. Taught skills
live in `$DOVE_HOME/taught/<name>/` (`~/.dove/taught/<name>/` by default); `teach_tool.py path <name>`
prints the folder.

There are three phases. Do them in order and do not skip the interview, even when the request
looks obvious: the rules and judgement calls the user knows are what make a skill reliable.

## 1. Interview

Ask short questions, a few at a time, and wait for answers. Cover:

1. **Goal**: what the task achieves and for whom, in one sentence. When should an agent use the skill,
   and when not.
2. **Inputs**: what changes from run to run (customer, date, file, repo, URL). These become
   variables written as `{{name}}`.
3. **Steps**: walk through the last time they did it. Ask "what did you check before that?" and
   "what would a new hire get wrong here?" for each step.
4. **Tools**: which apps, sites, CLIs, APIs or MCP tools are involved. Prefer the most direct route
   for each step: an API, CLI or MCP tool first (for example `gh`, a connected integration, a
   documented endpoint), then the browser, then desktop control. Note which routes the user
   allows.
5. **Rules and judgement**: thresholds, approvals, things that must never happen, how to handle the
   common exceptions. Quote the user's own words when they state a rule.
6. **Done when**: observable checks that prove success (a file exists with these fields, the PR is
   open and CI is green, the record shows status X). Avoid "looks right".
7. **A fresh example** to rehearse on later, with sample or test data, never real customer data
   or production accounts unless the user insists.

If the user pastes an SOP, runbook or transcript, read it first and ask only about the gaps.
If you can see the work (a repo, a recent thread, files), read them instead of asking what you can
find out yourself. Never ask for passwords, API keys or tokens and never write them into the skill;
refer to where the credential is configured instead ("the GitHub CLI is already signed in").

## 2. Write and validate

1. Pick a name: lowercase letters, digits and single hyphens, at most 64 characters, describing the
   task (`cut-release`, `weekly-invoice-reconcile`). Then run
   `uv run scripts/teach_tool.py new <name> --goal "<one-sentence goal>" --harness <your agent>`.
   It creates the folder with a template.
2. Replace every `{{TODO ...}}` in the new `SKILL.md`:
   - `description`: what it does **and when to use it**, with the words a user would say. This is
     how agents decide to load it, so be specific (at most 1024 characters).
   - `## Inputs`: each `{{variable}}`, what it means, an example, and where to get it if missing.
   - `## Steps`: numbered, imperative, one action per step, each with how to verify it worked.
     Name the exact command, endpoint, page or app control. Mark steps that change or send
     anything with **(confirm first)**.
   - `## Rules`: the user's rules and judgement calls.
   - `## Done when`: the observable checks.
   - `## Guardrails`: what never to do; when to stop and ask the user (sign-in walls, CAPTCHAs,
     2FA, payments, anything irreversible, anything that does not match the expected state).
   - Long reference material goes in `references/*.md`; reusable code goes in `scripts/`.
3. Write at least one eval in `evals/`: `evals/<case>.md` with **Input** (the fresh example),
   **Expected** (the done-when checks with concrete values) and **Setup** (test data or sandbox).
4. Run `uv run scripts/teach_tool.py validate <name>` and fix everything it reports. It checks the
   frontmatter, the required sections, leftover placeholders, evals, and secret-looking strings.
5. Show the user the finished `SKILL.md` and ask for corrections before you rehearse.

## 3. Rehearse (Practice)

A taught skill is trusted only after it works on a fresh example. Rehearse in a safe place: a
scratch branch or worktree, a test account, sample data, a separate browser space.

1. Read the skill folder as if you had never seen it, and follow `SKILL.md` exactly for the eval
   input. Do not use knowledge from the interview that is not written down: if you need it, the
   skill is missing it, so stop, fix `SKILL.md`, and start the rehearsal again.
2. Ask before any step marked **(confirm first)**, even in a sandbox.
3. Check every `## Done when` item and report each one as passed or failed with evidence.
4. Record the result:
   `uv run scripts/teach_tool.py rehearse <name> --pass --notes "<what you checked>" --harness <your agent>`
   or `--fail --notes "<what went wrong and what you changed>"`.
5. On a failure, improve `SKILL.md` (clearer step, missing rule, better check), validate again and
   rehearse again. Each fix is a lesson; write it into the skill, not only into your answer.
6. After the first passing rehearsal, run `uv run scripts/teach_tool.py publish <name>`. It copies the
   skill into the folders every agent reads (`~/.claude/skills`, `~/.agents/skills`), never
   overwriting a folder Dove did not create. The skill becomes **trusted** after two passing
   rehearsals (the count is in `TEACH.json`; `status <name>` shows it). Until then, tell the user
   to supervise its runs.

## Updating a taught skill

When the user corrects you while you are using a taught skill, or a run fails, offer to fix the
skill: edit `SKILL.md` in `$DOVE_HOME/taught/<name>/` (not the published copy), `validate`,
`rehearse`, then `publish` again. `teach_tool.py list` shows every taught skill with its status;
`unpublish <name>` removes the published copies.

## Commands

```sh
uv run scripts/teach_tool.py new <name> --goal "..." [--harness claude]
uv run scripts/teach_tool.py path <name>
uv run scripts/teach_tool.py validate <name>
uv run scripts/teach_tool.py rehearse <name> (--pass|--fail) --notes "..." [--harness codex] [--eval evals/x.md]
uv run scripts/teach_tool.py publish <name> [--force]
uv run scripts/teach_tool.py unpublish <name>
uv run scripts/teach_tool.py status <name>
uv run scripts/teach_tool.py list
```

`publish` refuses a skill that fails validation or has no passing rehearsal; `--force` skips the
rehearsal check only when the user explicitly asks for it.
