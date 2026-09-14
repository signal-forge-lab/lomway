# Public Generalization — Branch & Worktree Rollback Policy

Status: ACTIVE

Task: LMG-G0-02 (`docs/PUBLIC_GENERALIZATION_TASKS.yaml`).
Companion evidence: `docs/PUBLIC_GENERALIZATION_BASELINE.md` (English), `docs/PUBLIC_GENERALIZATION_BASELINE.ja.md` (Japanese).

## Purpose

Define the unambiguous rollback point and the branch/worktree discipline for the
public-generalization initiative, so that any LMG-G1-or-later change can be rolled back to
the verified pre-generalization baseline without endangering the current production
deployment.

## Protected state

- The current deployment is the six-namespace production profile recorded in
  `docs/PUBLIC_GENERALIZATION_BASELINE.md`.
- `config/proxy.local.toml`, SOPS material, and generated runtime state are machine-local.
  They stay gitignored, are never committed, and are never part of any rollback artifact.

## Stable rollback point

- The rollback point is the **root commit of `main`**: the initial baseline commit created
  from the captured pre-generalization tree before any LMG-G1 change is made.
- Identification is mechanical: `git rev-list --max-parents=0 main` must print exactly one
  hash; that hash is the rollback point.
- The commit is bound to the baseline evidence: a checkout of it must reproduce
  fmt/clippy cleanliness, the 0 + 10 + 5 + 1 test results, and a `--check` report of six
  backends.

## Protection status at capture time (2026-09-13)

- `main` was an unborn branch (zero commits). The entire pre-generalization tree existed
  only as the working tree/index state (43 staged `A`, 4 `AM`, 7 untracked files) and no
  stash existed. Until the baseline commit exists, that working tree is the only copy of
  the deployment.
- Interim hard rules until the baseline commit exists: no `git reset` of any kind, no
  discarding checkout (`checkout -- .`), no `git clean`, no `git stash drop/pop`, no branch
  deletion, no rebase.

## Branch policy

- All generalization work happens on work branches cut from the rollback point:
  `git switch -c public-generalization <rollback-point>`.
- `main` moves only at verified task-group boundaries and only by fast-forward; never
  force-push or rewrite `main` history.
- Machine-local configuration, SOPS material, and runtime state remain ignored forever.

## Activation step (operator, required before LMG-G1-01)

The baseline capture ran on an unborn branch, so making the rollback point immutable is one
deliberate commit, to be executed by the operator of record:

```text
git add -A
git commit -m "chore: pre-generalization baseline (LMG-G0 rollback point)"
git rev-list --max-parents=0 main   # must print exactly one hash
git status --porcelain              # must be empty
```

Activation record (2026-09-13, verification round 2): executed as commit
`c98ea957582006b781edfc6fb50e9f96368b156b` — `chore: pre-generalization baseline
(LMG-G0 rollback point)`. `git rev-list --max-parents=0 main` prints exactly this hash,
and the working tree was clean afterwards.

## Rollback procedure

1. Safety snapshot first: save `git status --porcelain` output outside the repository, and
   only if uncommitted work must survive inspection, `git stash push -u -m "pre-rollback
   safety snapshot"` (a safety stash is never dropped implicitly).
2. Discard generalization changes on a work branch with
   `git reset --hard <rollback-point>`, or abandon the branch entirely with
   `git switch main && git branch -D <branch>`. The root commit of `main` is never rewritten.
3. Re-verify restoration by re-running the G0 baseline suite (fmt, clippy, `policy`,
   `proxy`, `real_backends --ignored`, `--check`) and comparing against
   `docs/PUBLIC_GENERALIZATION_BASELINE.md` (16 passing test functions, six namespaces).
4. A rollback that cannot reproduce the recorded baseline evidence is not complete; stop
   and investigate before any further change.
