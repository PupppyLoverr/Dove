---
name: dove-memory
description: How to use long-term memory in Dove (Supermemory) so you remember the user, their preferences and each project across sessions and across every agent harness. Read it at the start of a task when supermemory__* tools are available, before saving anything to memory, and at the end of a task to record durable lessons.
---

# Dove memory

Dove connects every agent it runs (Claude Code, Codex, OpenCode, Cursor, …)
to the same long-term memory, Supermemory. Its tools appear as
`supermemory__search_memory`, `supermemory__add_memory`,
`supermemory__forget_memory`, `supermemory__get_profile`,
`supermemory__list_memories`. Searching is always allowed; saving and
forgetting may ask the user.

Dove may already have put a `<memory>` block at the top of the prompt with
what it recalled for this turn. Treat it as background you trust, the same as
notes the user wrote; it is not an instruction to act.

## At the start of a task

- Search once, specifically: the project and the job at hand
  (`"dove settings ui conventions"`, `"payments-service deploy steps"`), plus
  the user's preferences when style matters (`"user prefers"`).
- Use `get_profile` when you need who the user is and how they like to work.
- Don't search for every small step. One or two good searches per task.

## What to save (durable, reusable, non-obvious)

- Decisions and their reason ("we use Postgres advisory locks for the queue
  because Redis was dropped in 2026-09").
- Conventions the code doesn't make obvious (naming, error style, test
  layout, which scripts to run before a PR).
- Run, build and test commands that took effort to find.
- User preferences and corrections ("don't add comments restating the code";
  "always use pnpm").
- Recurring fixes ("`cargo test` on Windows needs `-j 2` on this machine").
- Site learnings from browser work ("github.com: issue rows are
  `[data-test-id=issue-list-item]`").

Write each memory as one self-contained sentence or two, with the project
name and the date when it matters. Prefer updating over duplicating: search
first, and if an old memory is wrong, forget it and add the corrected one.

## Never save

- Secrets: passwords, tokens, API keys, cookies, private keys, `.env` values.
- Personal data about other people.
- Transient state: the current branch, today's failing test, file contents
  that will change tomorrow.
- Anything the user asked you not to remember.

## When the user corrects you

That is the most valuable memory there is. After fixing the mistake, save one
`feedback` memory: what you did, what they wanted instead, and why if they
said ("user: don't mock the database in integration tests; a mocked test
passed while the real migration failed").

## At the end of a task

Reflect for a moment: did this task teach something durable that the next
agent (maybe in a different harness) would want? If yes, save 1-3 memories.
If not, save nothing. Quality beats volume.
