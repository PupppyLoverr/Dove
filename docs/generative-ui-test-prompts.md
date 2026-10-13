# Generative UI and agent-browser test prompts

Use these in any Dove thread. Claude Code gets the `dove` MCP tools
automatically; Codex and ACP agents do too. Each prompt says what to expect.

## Interactive cards (`show_ui`)

1. **Dashboard**
   > Use show_ui to build a dashboard of this repo: stat tiles for files, lines of Rust, and crates, a bar chart of lines per crate, and a breakdown of languages. Give it the id "repo-stats".

   Expected: one card with stats, bars and a stacked breakdown bar.

2. **Live update in place**
   > Now recount and call show_ui again with the same id "repo-stats", adding a callout with the biggest file.

   Expected: the same card updates; no second card appears.

3. **Decision with option cards**
   > I need to pick a state-management approach for a new GPUI panel. Show me 3 options as cards (with a "recommended" badge on one) plus a timeline of what each step of the migration would be, and buttons to pick.

   Expected: selectable cards. Clicking one and pressing the button sends your choice back as a message.

4. **Tunable settings**
   > Help me plan a load test. Show a card with a choice for the environment (staging/production), a number stepper for concurrent users (10-500, step 10), a toggle for "ramp up", and a checklist of pre-flight checks, then a "Run plan" button.

   Expected: you change the values, press the button, and the agent receives every value.

5. **Tabs**
   > Show a card with tabs "Latency", "Errors" and "Throughput", each with a chart of made-up data for the last 7 days.

   Expected: switching tabs swaps the charts inside the card.

6. **Alter a card**
   Hover any card and press **Change**. The composer fills with `Change the "<title>" card: `; type e.g. "make it a line chart" and send.

## Rich chat rendering

7. > Explain quicksort with a mermaid flowchart, the average-case complexity as display math, and the recurrence $T(n) = 2T(n/2) + O(n)$ inline.

8. > Show the quadratic formula, the sum of the first n integers, and the Gaussian integral as display math.

9. > Write a markdown table comparing Rust, Go and Zig on 5 criteria, then a task list of next steps.

## In-app browser (any harness)

10. > Open https://example.com in the in-app browser, take a snapshot, click "Learn more", and tell me the page title.

11. > Start the dev server for this project, open it in the in-app browser, take a screenshot, and tell me what you see.

12. > Go to https://news.ycombinator.com, read the top 5 titles, then show them to me in a show_ui table.

Expected: the side panel opens on the browser. A purple "Agent" cursor moves and clicks, the toolbar shows a pill saying what the agent is doing, and screenshots come back to the agent.

## Navigation and speed

13. Have a thread with 3 or more prompts:
    - Hover the marks on the chat's left edge to preview each prompt and reply; click one to glide to it.
    - Use **Alt+↑ / Alt+↓** to step between prompts.
    - Use **PageUp / PageDown** and **Ctrl+Home / Ctrl+End** to scroll.
    - Drag the scrollbar on the right.

14. On the Home screen with Claude Code selected and Worktree off, start typing, wait a few seconds, then send. The first reply arrives much sooner: about 3 s instead of about 11 s in a local measurement, because the agent was started while you typed.
