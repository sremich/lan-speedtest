# [PROJECT NAME] — Initial Prompt

<!--
HOW TO USE THIS TEMPLATE
- This project was started from the `project-scaffold` template repo
  (github.com/sremich/project-scaffold). The CI workflows, .gitignore,
  CHANGELOG, VERSION file, doc skeletons, and release automation ALREADY
  EXIST — do not rebuild them. Milestone 0 is resolving the scaffold's
  TODO(milestone-0) markers, not creating scaffolding from scratch.
- Fill in every [BRACKETED] placeholder; delete any section that doesn't
  apply. Lines marked "TIP:" are guidance for the human filling this in —
  delete them from the finished prompt.
- Save the filled-in copy as source-files/initial-prompt.txt (that folder is
  gitignored), then start the agent with:
  "please look at the 'initial-prompt.txt' in the source-files folder for
  the initial prompt"
-->

## 1. Context

[Who you are in this context and why this exists. One or two paragraphs of
plain-language background. Example: "Twice a year I work on a tradeshow
that utilizes Bitfocus Companion to control 10–30 remote PCs running
PowerPoint."]

As context, the current workflow is the following:

1. [Step-by-step description of how things work TODAY, before this tool
   exists. Number each step. Name the real systems, protocols, and hardware
   involved (OSC, NDI, SDI, MediorNet, Technitium, Proxmox, etc.).]
2. [...]

## 2. Things to note

1. [Constraints, quirks, and gotchas the agent can't discover on its own —
   especially known bugs/limitations in third-party systems and the
   workarounds you already use. These have been the highest-value lines in
   past prompts (e.g. "the MediorNet integration hangs if the route already
   exists, so I use a variable + rule instead of a direct take").]
2. [Existing infrastructure and addresses it will talk to.]
3. [Anything that must NOT happen, e.g. "captions must not be burned into
   the video", "never authenticate to the production routers".]
4. Make sure to look through the `source-files/` directory. I've added:
   [list each file and what it is — configs, exports, sample data, the
   actual files the tool must work against].

## 3. Problem

1. [The pain, stated plainly. What is slow, tedious, manual, error-prone,
   or impossible today? This is the "why", separate from the feature list.]

## 4. Want

<!-- TIP: Phrase each want as an acceptance criterion — add a "Done when:"
     line describing a test you could actually perform. "I'd like to edit
     an Island and have it propagate" is good; "Done when: I change X on an
     Island and every attached panel shows it, except detached keys" is
     better. These become the skeleton of the milestone plan.
     TIP: Tag each want [must] / [should] / [later] so the agent slices
     milestones the way you would. -->

- [must] [Deliverable in one line, including form factor: e.g. "a
  Docker-based web UI that...", "a service in a container on a Linux VM".]
- [must] [Each concrete capability as its own bullet. Describe the workflow
  you want, not the implementation.]
  - Done when: [the observable test that proves it works].
- [should] [...]
  - Done when: [...]
- [later] [...]
- [If there's a web UI: it needs login auth with credentials changeable in
  a settings page, and the version + git SHA visible in a corner of the
  page (the scaffold injects these at build time — wire them through).]

## 5. Non-goals

<!-- TIP: Stating what is explicitly OUT of scope prevents drift and gives
     the agent a place to park ideas. Anything suggested mid-project that
     isn't in section 4 goes to HANDOVER.md's open-items list, not into the
     current milestone. -->

- [Things this project deliberately will not do, at least before 1.0.0.]
- [Ideas you already know about but are parking, so the agent doesn't
  re-propose or quietly build them.]

## 6. Integrations

- [Each external system it must talk to, one bullet each, with what the
  integration should do and any known API details/versions.]
- [Note which integrations you can provide live test access to, and which
  must be mocked/faked in tests.]

## 7. Environment

- Target runtime: [Docker container on a Linux VM / etc.]. The scaffold's
  `docker-compose.yml` and `.env.example` are stubs — keep them current as
  real config appears.
- Dev machine is Windows 11 (PowerShell 5.1 quirks apply); builds and
  container testing happen in [WSL Debian / remote VM / GitHub CI].
- Test targets I can provide: [live instance at IP / hardware on site /
  throwaway VM]. Credentials will be supplied separately at test time —
  they go in `.env` / local config only, never in the repo, chat, or docs.
- [Anything only reachable on site / infrequently — say so, since it gates
  what "verified" can mean before 1.0.0.]

## 8. Rules — repo, versioning, releases

- This repo was created from the `project-scaffold` template. **Milestone 0
  = resolve every `TODO(milestone-0)` marker** (CI test command, Dockerfile,
  compose file, .env.example, doc skeletons copied into place from
  `templates/`). The release workflow refuses to run while any marker
  remains — that is intentional; fix the marker, never the check.
- Versioning: bugfixes are +0.0.1, minor feature releases are +0.1.0, major
  feature releases or reworks are +1.0.0. **Always three-part X.Y.Z** (never
  "v0.4"). The version lives ONLY in the `VERSION` file; if the project's
  language needs it elsewhere (package.json, `__init__.py`), that copy is
  generated or read from `VERSION`, never hand-edited.
- **CI owns releases end to end.** A release is: update CHANGELOG (move
  Unreleased → version), bump `VERSION`, commit, push, push annotated tag
  `vX.Y.Z`. CI then verifies tag == VERSION, runs tests, builds and pushes
  the image to GHCR (with version + git SHA baked in), and creates the
  GitHub release — pre-release automatically while the version is 0.x. If
  any step fails, no partial release exists. **Never create releases, push
  images, or edit tags by hand.**
- After CI goes green: bump the compose image pin, and complete the release
  checklist in `CLAUDE.md` — paste it into the conversation and tick every
  item. Checklists survive context loss; memory doesn't.
- Before writing any code: **restate your understanding** of my system and
  workflow back to me in your own words, ask clarifying questions, THEN
  propose a milestone plan (0.1.0, 0.2.0, … 1.0.0) built from the tagged
  wants in section 4, and let me approve it.
- Work one milestone at a time: build → test → release → audit. **Expect me
  to checkpoint between milestones; never start the next one unprompted.**
- Never commit secrets, tokens, runtime logs, or `data/`. `source-files/`,
  `HANDOVER.md`, `CLAUDE.md`, and `DECISIONS.md` are gitignored — they live
  only in this OneDrive-synced folder.
- OneDrive discipline: commit + push before I switch machines (GitHub is
  the source of truth, not OneDrive); keep `.git`, venvs, and `data/`
  pinned "always keep on this device"; on a new machine recreate the
  venv/node_modules rather than trusting the synced copy; prefer building
  out-of-tree to avoid sync churn.

## 9. Rules — documentation and handover

- Haiku writes the README and CHANGELOG; Sonnet audits them. Run this pair
  before every release.
- Haiku also builds and updates a wiki (exhaustive, easy to use); Sonnet
  audits it. Where it lives depends on repo visibility:
  - **Private repo → the wiki lives IN the repo**, as markdown files under
    `docs/wiki/` with `Home.md` as the index page. Do not use GitHub's wiki
    section on a private repo.
  - **Public repo → use GitHub's wiki section.** Gotcha: GitHub has no wiki
    API — the `.wiki.git` repo does not exist until the first page is
    created by hand in the web UI; ask me to do that once, early. If a
    private repo later goes public, migrate `docs/wiki/` there.
  - Either way, write it public-safe from day one (no real IPs, hostnames,
    or credentials) — private repos have gone public before. Update at
    least the release-history page on every release.
- Copy `templates/HANDOVER.md` to the repo root during milestone 0 and keep
  it current: it is the roaming context document for agents on any of my
  machines (local agent memory doesn't follow me; this file does). It
  contains a **Cold start** section — the exact commands from fresh machine
  to running dev environment — and a **session-close checklist** at the
  top. When I say I'm wrapping up, run that checklist explicitly and show
  me the ticks. Update HANDOVER.md before ending ANY session that changed
  anything.
- Copy `templates/DECISIONS.md` to the repo root during milestone 0.
  Decisions, chosen trade-offs, and hard-won lessons are **append-only**
  entries there — never rewritten, never trimmed. HANDOVER.md churns every
  session; DECISIONS.md only grows. Cross-reference instead of duplicating.
- Copy `templates/CLAUDE.md` to the repo root during milestone 0 and grow
  it with the project's binding engineering rules as they emerge. It also
  holds the release checklist and the testing-tier table.

## 10. Rules — testing and validation

The four testing tiers, and what each gates:

| Tier | What it is | Gates |
|------|-----------|-------|
| 1. Unit | Fast tests, no external systems — in-memory fakes of every integration surface | Every push (CI) and every Docker build |
| 2. e2e | Real components talking to each other locally (containers, loopback) | Any change to core pipeline / protocol code |
| 3. Live | Validation against a real instance of [external system] | Any change to integration/communication code, before it ships |
| 4. Hardware / on-site | [The verification only I can do] | **1.0.0.** Until it passes, every release stays a pre-release |

- Keep manual testing I have to do myself to a minimum until just before
  1.0.0.
- Verify before claiming: tier 1 always; the matching higher tier whenever
  its trigger applies; actually run the container when the Dockerfile or
  entrypoint changes.

---

Restate the system as you understand it, ask your clarifying questions, and
then propose the milestone plan.
