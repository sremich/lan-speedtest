# [PROJECT NAME] — agent handover context

> **STANDING REQUIREMENT — read this first.** This document is the roaming
> context for AI agents working on this project from any of Stephen's
> machines. It is deliberately **gitignored** and lives only in the
> OneDrive-synced project directory. Local agent memory
> (`~/.claude/projects/...`) does not follow Stephen between machines; this
> file is what does. **Any agent that makes a meaningful change — a release,
> a design decision, an environment change, a new pending item — must update
> this document before ending the session.**
>
> _Last updated: [DATE] — [one-line summary of last session]_

## Session-close checklist

Run this explicitly (and show the ticks) whenever Stephen says they're
wrapping up, and before ending any session that changed anything:

- [ ] Tests green (tier 1 minimum; higher tiers if their triggers applied)
- [ ] All work committed AND pushed (GitHub is the source of truth)
- [ ] `CHANGELOG.md` Unreleased section reflects this session's changes
- [ ] New decisions / hard-won lessons appended to `DECISIONS.md`
- [ ] This file updated: Current state, Open items, Session log, _Last updated_
- [ ] If a release happened: compose pin bumped, CLAUDE.md release checklist completed

## What this project is

[Two or three sentences: what it does, the stack, where it runs, the repo
URL, where images are published.]

## Current state

- Version: [vX.Y.Z] — local main == origin/main == tag? [yes/no]
- Gates: [test command] → [result]
- [Anything in flight or half-done.]

## Cold start (fresh machine → productive)

Exact commands, no prose gaps. A new agent on a new machine should be
running in minutes using only this section.

1. `git clone [repo-url]` (or open the OneDrive folder — then pin `.git`,
   venvs, and `data/` "always keep on this device")
2. [Recreate the environment — venv / node_modules / SDK paths. Never trust
   a synced venv.]
3. [How to run the tests.]
4. [How to run the app / container locally.]
5. [Where credentials come from (`.env` — ask Stephen; never in chat/repo).]

## Environment map

[The systems this project talks to: names, addresses, versions, which are
production (document-only, never authenticate) vs test targets. What only
exists on site.]

## Open items / next steps

1. [Ordered list. Parked ideas from mid-project conversations land here,
   not in the current milestone.]

## Session log (condensed)

- **[DATE]:** [What happened, what shipped, what broke, what was decided.]
