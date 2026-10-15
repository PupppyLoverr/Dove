"""Seed a scratch DOVE_HOME for a perf run.

The home must already hold a dove.db created by a short Dove launch (so the
schema is whatever this build creates). This adds one project, points the
fake agent at Claude Code and three ACP agents, and schedules N automations
that each start one session on the scheduler's first tick.

usage: seed_home.py <dove_home> <project_dir> <fake_agent_exe> <sessions>
"""

import sqlite3
import sys
import time
import uuid
from datetime import datetime, timedelta
from pathlib import Path

home, project, fake, n = Path(sys.argv[1]), Path(sys.argv[2]), Path(sys.argv[3]), int(sys.argv[4])
# Mixed harnesses: Claude Code's stream-json driver plus three ACP agents.
HARNESSES = ["claude_code", "gemini", "qwen", "copilot"]

con = sqlite3.connect(str(home / "dove.db"))
now = int(time.time() * 1000)


def setting(key: str, value: str) -> None:
    con.execute(
        "INSERT INTO settings(key, value) VALUES(?, ?) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        (key, value),
    )


row = con.execute("SELECT id FROM projects WHERE path = ?", (str(project),)).fetchone()
if row:
    project_id = row[0]
else:
    project_id = str(uuid.uuid4())
    con.execute(
        "INSERT INTO projects(id, name, path, created_at) VALUES(?, ?, ?, ?)",
        (project_id, project.name, str(project), now),
    )
setting("active_project", project_id)
setting("intro.seen", "1")
setting("use_worktree", "false")
for h in HARNESSES:
    setting(f"agent_path.{h}", str(fake))

# An `at` trigger one minute in the past, on an automation created two
# minutes ago, is due exactly once: on the scheduler's first tick.
fire = datetime.now() - timedelta(minutes=1)
con.execute("DELETE FROM automations")
for i in range(n):
    harness = HARNESSES[i % len(HARNESSES)]
    trigger = f'{{"type":"at","hour":{fire.hour},"minute":{fire.minute},"weekdays":[]}}'
    con.execute(
        "INSERT INTO automations(id, name, prompt, project_id, harness, model, trigger, worktree, enabled,"
        " last_run, state, created_at) VALUES(?, ?, ?, ?, ?, NULL, ?, 0, 1, NULL, '{}', ?)",
        (str(uuid.uuid4()), f"perf {i + 1} ({harness})", "Stream a long answer with tool calls.", project_id,
         harness, trigger, now - 120_000),
    )
con.commit()
con.close()
print(f"seeded {n} automation(s) in {home}")
