#!/usr/bin/env python3
# /// script
# requires-python = ">=3.9"
# dependencies = []
# ///
"""Create, validate, rehearse and publish skills taught to Dove. See ../SKILL.md."""
import argparse
import datetime
import hashlib
import json
import os
import re
import shutil
import sys
from pathlib import Path

NAME_RE = re.compile(r"^[a-z0-9]+(-[a-z0-9]+)*$")
# Dove's bundled skills; a taught skill may not shadow them.
RESERVED = {"pdf", "pptx", "xlsx", "docx", "teach"}
SECTIONS = ["## Inputs", "## Steps", "## Rules", "## Done when", "## Guardrails"]
TRUST_AFTER = 2
MARKER = ".dove-skill"
SECRETS = [
    re.compile(r"sk-(?:ant-|proj-)?[A-Za-z0-9_\-]{20,}"),
    re.compile(r"gh[pousr]_[A-Za-z0-9]{30,}"),
    re.compile(r"github_pat_[A-Za-z0-9_]{30,}"),
    re.compile(r"AKIA[0-9A-Z]{16}"),
    re.compile(r"xox[abprs]-[A-Za-z0-9-]{10,}"),
    re.compile(r"AIza[0-9A-Za-z_\-]{35}"),
    re.compile(r"-----BEGIN [A-Z ]*PRIVATE KEY-----"),
    re.compile(r"(?i)\b(password|passwd|secret|api[_-]?key|access[_-]?token)\b\s*[:=]\s*['\"]?[A-Za-z0-9/+_\-]{12,}"),
]

TEMPLATE = """---
name: {name}
description: {{{{TODO what this skill does and when to use it, in the words a user would say}}}}
---

# {title}

Goal: {goal}

## Inputs

- `{{{{TODO variable}}}}`: {{{{TODO what it is, an example, where to find it if missing}}}}

## Steps

1. {{{{TODO first action, and how to verify it worked}}}}

## Rules

- {{{{TODO the user's rules and judgement calls, in their words}}}}

## Done when

- {{{{TODO observable check that proves success}}}}

## Guardrails

- Never type passwords, API keys, payment or ID details; never copy secrets into files or answers.
- Stop and ask the user at sign-in walls, CAPTCHAs, 2FA prompts, payments, and before anything
  irreversible (send, submit, publish, delete, merge, deploy) unless this skill says it is approved.
- If what you see does not match what a step expects, stop and report instead of guessing.
- {{{{TODO task-specific limits}}}}
"""

EVAL_TEMPLATE = """# {{{{TODO case name}}}}

**Setup:** {{{{TODO test data, sandbox branch or test account to use}}}}

**Input:** {{{{TODO values for each variable}}}}

**Expected:** {{{{TODO the done-when checks with concrete values}}}}
"""


def dove_home():
    env = os.environ.get("DOVE_HOME")
    return Path(env) if env else Path.home() / ".dove"


def taught_root():
    return dove_home() / "taught"


def harness_dirs():
    claude = os.environ.get("CLAUDE_CONFIG_DIR")
    claude = Path(claude) if claude else Path.home() / ".claude"
    return [claude / "skills", Path.home() / ".agents" / "skills"]


def now():
    return datetime.datetime.now(datetime.timezone.utc).replace(microsecond=0).isoformat()


def check_name(name):
    if not NAME_RE.match(name) or len(name) > 64:
        sys.exit(f"Invalid name {name!r}: use lowercase letters, digits and single hyphens (at most 64).")
    if name in RESERVED:
        sys.exit(f"{name!r} is one of Dove's bundled skills; pick another name.")


def skill_dir(name):
    check_name(name)
    return taught_root() / name


def existing(name):
    d = skill_dir(name)
    if not (d / "SKILL.md").is_file():
        sys.exit(f"No taught skill named {name!r} in {taught_root()}. Run `new` first.")
    return d


def load_meta(d):
    try:
        return json.loads((d / "TEACH.json").read_text(encoding="utf-8"))
    except (OSError, ValueError):
        return {"name": d.name, "rehearsals": []}


def save_meta(d, meta):
    tmp = d / "TEACH.json.tmp"
    tmp.write_text(json.dumps(meta, indent=2) + "\n", encoding="utf-8")
    os.replace(tmp, d / "TEACH.json")


def status_of(meta):
    runs = meta.get("rehearsals", [])
    passes = sum(1 for r in runs if r.get("passed"))
    if not runs:
        return "draft"
    if runs[-1].get("passed") and passes >= meta.get("trust_after", TRUST_AFTER):
        return "trusted"
    return "rehearsing"


def summary(d):
    meta = load_meta(d)
    runs = meta.get("rehearsals", [])
    return {
        "name": d.name,
        "goal": meta.get("goal", ""),
        "status": status_of(meta),
        "passes": sum(1 for r in runs if r.get("passed")),
        "fails": sum(1 for r in runs if not r.get("passed")),
        "trust_after": meta.get("trust_after", TRUST_AFTER),
        "published": bool(meta.get("published")),
        "version": meta.get("version", 1),
        "path": str(d),
    }


def frontmatter(text):
    if not text.startswith("---\n"):
        return None
    end = text.find("\n---", 4)
    if end < 0:
        return None
    fields = {}
    for line in text[4:end].splitlines():
        if ":" in line and not line.startswith((" ", "\t")):
            k, v = line.split(":", 1)
            fields[k.strip()] = v.strip()
    return fields


def problems(d):
    """Everything that keeps the skill from being published, as readable lines."""
    out = []
    text = (d / "SKILL.md").read_text(encoding="utf-8")
    fm = frontmatter(text)
    if fm is None:
        out.append("SKILL.md must start with YAML frontmatter between --- lines.")
        fm = {}
    if fm.get("name") != d.name:
        out.append(f"frontmatter name must be {d.name!r} (the folder name), found {fm.get('name')!r}.")
    desc = fm.get("description", "")
    if not desc or "{{TODO" in desc:
        out.append("frontmatter description is missing: say what the skill does and when to use it.")
    elif len(desc) > 1024:
        out.append(f"description is {len(desc)} characters; the limit is 1024.")
    elif len(desc) < 40:
        out.append("description is too short for an agent to know when to use the skill.")
    for section in SECTIONS:
        if not re.search(rf"^{re.escape(section)}\s*$", text, re.M):
            out.append(f"missing section {section!r}.")
    steps = re.search(r"^## Steps\s*$(.*?)(?=^## |\Z)", text, re.M | re.S)
    if steps and not re.search(r"^\s*1\.\s+\S", steps.group(1), re.M):
        out.append("## Steps needs a numbered list (1. ...).")
    evals = sorted((d / "evals").glob("*.md")) if (d / "evals").is_dir() else []
    if not evals:
        out.append("add at least one eval in evals/*.md (Setup, Input, Expected).")
    for path in sorted(p for p in d.rglob("*") if p.is_file() and p.suffix in {".md", ".txt", ".json", ".py", ".sh"}):
        rel = path.relative_to(d).as_posix()
        if rel in {"TEACH.json", MARKER} or rel.endswith(".tmp"):
            continue
        body = path.read_text(encoding="utf-8", errors="replace")
        if "{{TODO" in body:
            out.append(f"{rel}: replace every {{{{TODO ...}}}} placeholder.")
        for pat in SECRETS:
            if pat.search(body):
                out.append(f"{rel}: contains something that looks like a secret; remove it and refer to where the credential is configured.")
                break
    return out


def stamp(d):
    h = hashlib.sha256()
    for path in sorted(p for p in d.rglob("*") if p.is_file()):
        rel = path.relative_to(d).as_posix()
        if rel in {"TEACH.json", MARKER} or rel.endswith(".tmp"):
            continue
        h.update(rel.encode())
        h.update(path.read_bytes())
    return f"dove taught {h.hexdigest()[:16]}\n"


def owned(dest):
    """A published folder Dove may replace: it carries Dove's marker."""
    try:
        return (dest / MARKER).read_text(encoding="utf-8").startswith("dove taught ")
    except OSError:
        return False


def cmd_new(a):
    d = skill_dir(a.name)
    if d.exists():
        sys.exit(f"{d} already exists. Edit it, or pick another name.")
    (d / "evals").mkdir(parents=True)
    title = a.name.replace("-", " ").capitalize()
    (d / "SKILL.md").write_text(TEMPLATE.format(name=a.name, title=title, goal=a.goal.strip()), encoding="utf-8")
    (d / "evals" / "example.md").write_text(EVAL_TEMPLATE, encoding="utf-8")
    meta = {
        "name": a.name,
        "goal": a.goal.strip(),
        "created": now(),
        "taught_with": a.harness or "",
        "version": 1,
        "trust_after": TRUST_AFTER,
        "published": False,
        "rehearsals": [],
    }
    save_meta(d, meta)
    print(json.dumps({"created": str(d), "edit": [str(d / "SKILL.md"), str(d / "evals" / "example.md")]}, indent=2))


def cmd_path(a):
    print(skill_dir(a.name))


def cmd_validate(a):
    d = existing(a.name)
    issues = problems(d)
    if issues:
        print(f"{a.name}: {len(issues)} problem(s)")
        for i in issues:
            print(f"- {i}")
        sys.exit(1)
    print(f"{a.name}: valid")


def cmd_rehearse(a):
    d = existing(a.name)
    meta = load_meta(d)
    meta.setdefault("rehearsals", []).append(
        {
            "at": now(),
            "passed": bool(a.passed),
            "harness": a.harness or "",
            "eval": a.eval or "",
            "notes": a.notes.strip(),
            "stamp": stamp(d).split()[-1],
        }
    )
    save_meta(d, meta)
    s = summary(d)
    hint = ""
    if a.passed and not s["published"]:
        hint = " Run `publish` to make it available to every agent."
    elif not a.passed:
        hint = " Improve SKILL.md, validate, and rehearse again."
    print(f"{a.name}: {s['status']} ({s['passes']} passed, {s['fails']} failed, trusted after {s['trust_after']}).{hint}")


def cmd_publish(a):
    d = existing(a.name)
    issues = problems(d)
    if issues:
        print("Fix these first (`validate` lists them):")
        for i in issues:
            print(f"- {i}")
        sys.exit(1)
    meta = load_meta(d)
    if not any(r.get("passed") for r in meta.get("rehearsals", [])) and not a.force:
        sys.exit("No passing rehearsal yet. Rehearse on a fresh example first (or --force if the user asked).")
    mark = stamp(d)
    done, kept = [], []
    for root in harness_dirs():
        dest = root / a.name
        if dest.exists() and not owned(dest):
            kept.append(str(dest))
            continue
        if dest.exists():
            shutil.rmtree(dest)
        shutil.copytree(d, dest, ignore=shutil.ignore_patterns("TEACH.json", "*.tmp", MARKER))
        (dest / MARKER).write_text(mark, encoding="utf-8")
        done.append(str(dest))
    if done:
        meta["published"] = True
        meta["published_at"] = now()
        meta["version"] = meta.get("version", 1) + (1 if meta.get("published_stamp") not in (None, mark.split()[-1]) else 0)
        meta["published_stamp"] = mark.split()[-1]
        save_meta(d, meta)
    print(json.dumps({"published": done, "kept_existing_not_doves": kept, "status": status_of(meta)}, indent=2))
    if not done:
        sys.exit(1)


def cmd_unpublish(a):
    d = existing(a.name)
    removed = []
    for root in harness_dirs():
        dest = root / a.name
        if dest.exists() and owned(dest):
            shutil.rmtree(dest)
            removed.append(str(dest))
    meta = load_meta(d)
    meta["published"] = False
    save_meta(d, meta)
    print(json.dumps({"removed": removed}, indent=2))


def cmd_status(a):
    d = existing(a.name)
    s = summary(d)
    s["problems"] = problems(d)
    s["rehearsals"] = load_meta(d).get("rehearsals", [])[-5:]
    print(json.dumps(s, indent=2))


def cmd_list(_a):
    root = taught_root()
    items = []
    if root.is_dir():
        items = [summary(p) for p in sorted(root.iterdir()) if (p / "SKILL.md").is_file()]
    print(json.dumps(items, indent=2))


def main(argv=None):
    p = argparse.ArgumentParser(description=__doc__)
    sub = p.add_subparsers(dest="cmd", required=True)
    s = sub.add_parser("new", help="create a taught skill from the template")
    s.add_argument("name")
    s.add_argument("--goal", required=True)
    s.add_argument("--harness")
    s.set_defaults(fn=cmd_new)
    for name, fn in [("path", cmd_path), ("validate", cmd_validate), ("unpublish", cmd_unpublish), ("status", cmd_status)]:
        s = sub.add_parser(name)
        s.add_argument("name")
        s.set_defaults(fn=fn)
    s = sub.add_parser("rehearse", help="record a rehearsal result")
    s.add_argument("name")
    g = s.add_mutually_exclusive_group(required=True)
    g.add_argument("--pass", dest="passed", action="store_true")
    g.add_argument("--fail", dest="passed", action="store_false")
    s.add_argument("--notes", required=True)
    s.add_argument("--harness")
    s.add_argument("--eval")
    s.set_defaults(fn=cmd_rehearse)
    s = sub.add_parser("publish", help="copy the skill to every agent's skills folder")
    s.add_argument("name")
    s.add_argument("--force", action="store_true")
    s.set_defaults(fn=cmd_publish)
    s = sub.add_parser("list")
    s.set_defaults(fn=cmd_list)
    a = p.parse_args(argv)
    a.fn(a)


if __name__ == "__main__":
    main()
