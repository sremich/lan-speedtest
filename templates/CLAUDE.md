# [PROJECT NAME] — working notes

[One paragraph: what this is, stack, where it runs.]

## Commands

```sh
# TODO(milestone-0): the real commands
# [install]
# [lint]
# [test]
```

## Testing tiers

| Tier | What it is | Gates |
|------|-----------|-------|
| 1. Unit | Fast, no external systems; in-memory fakes of integration surfaces | Every push (CI), every Docker build |
| 2. e2e | Real components locally (containers, loopback) | Core pipeline / protocol changes |
| 3. Live | Against a real [external system] instance | Any integration/communication change, before it ships |
| 4. Hardware / on-site | [the verification only Stephen can do] | 1.0.0; until then all releases are pre-releases |

Verify before claiming: tier 1 always; the matching higher tier when its
trigger applies; run the actual container when Dockerfile/entrypoint change.

## Release checklist

Paste this into the conversation and tick every item, in order, for every
release. CI does the middle steps itself — the checklist is how we prove
nothing around them was skipped.

- [ ] Tier-1 tests green locally; higher tiers if triggered this cycle
- [ ] Haiku wrote/updated README + CHANGELOG; Sonnet audited them
- [ ] `CHANGELOG.md`: Unreleased → new `vX.Y.Z` section
- [ ] `VERSION` bumped (three-part X.Y.Z, correct increment: +0.0.1 / +0.1.0 / +1.0.0)
- [ ] Commit pushed; annotated tag `vX.Y.Z` pushed
- [ ] CI release workflow fully green (verify → build+push → release created)
- [ ] Pre-release flag correct (automatic while 0.x)
- [ ] `docker-compose.yml` image pin bumped to the new tag; committed
- [ ] Wiki updated (at minimum Release-History); Sonnet audited; public-safe.
      Location: `docs/wiki/` in-repo while the repo is private; GitHub wiki
      section once public
- [ ] `HANDOVER.md` + `DECISIONS.md` updated

## House conventions

- Commits end with `Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>`;
  git identity is repo-local `Stephen <sremich@gmail.com>`.
- The version lives ONLY in `VERSION`; language-level copies are derived,
  never hand-edited. CI refuses tags that disagree.
- Never commit secrets, tokens, runtime logs, or `data/`. Credentials
  arrive via `.env`, not chat.
- [Project-specific binding rules accumulate here as they emerge — schema
  compatibility promises, ownership guards, escaping rules, etc.]
