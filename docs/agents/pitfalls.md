# Pitfalls for agents on the open issues

Read this before you start an open issue. Each item cost a previous agent one or more rounds.
Terms in **bold** are defined in `GLOSSARY.md`.

## 1. Read the handoff first, not the log

`docs/progress-log.md` (500 KB), `docs/gaps.md` (666 KB) and `docs/porting-roadmap.md` (188 KB) are too
large to read in full. Do not try.

- The current state of an issue is its latest **handoff** comment: `gh issue view <n> --comments`.
- In the roadmap, read only these sections: Principles, "How to port one thing", Triage, "Rules for the
  next agent or contributor".
- Search the log for the issue number (`issue #26`) only when the handoff points you to a round.

When you stop work on an issue, post a new handoff comment with these three headings:

1. **Current state**: what is true on which commit, with the numbers.
2. **Rejected variants**: each change that was tried and reverted, with its result. Do not retry these.
3. **Next single step**: one change, and the legs to re-gate after it.

Continue to append the round to the progress log. The log is history. The handoff is the state.

## 2. The walk legs are regression checks, not blockers

Since #23, the default walk **gate** passes 9 of 9 **legs** and `--walk all` is green. The legs are
3-second retail captures with ±25-40 % time quantisation. Use them to catch a regression, compared
leg by leg with the baseline build. Do not tune code to move one leg back inside its tolerance.

Every open issue has the label `ready-for-agent`, so the label does not tell you which issue to start.
The blockers do. The order is #38 (cog VM audit), then #39 (levels 2-5 exit chains, ported from the
decomp), then #28 (the levels 2-5 routes, final acceptance only). #30 (level-1 route) and #31
(collision chain as one unit) can run in parallel. #26 and #27 were closed into #31: they were
per-leg collision buckets, and per-leg fixes only moved failures to other legs. Find engine
mismatches by reading the decomp, not by hand-authoring routes: a route finds one mismatch per many
runs and invites routing around it (#28's first round found #35 that way; one `--cog-check` pass
against the engine's executor then showed the whole class).

- Before you start an issue, count its open blockers. Start only when the count is 0:
  `gh api repos/{owner}/{repo}/issues/<n> --jq .issue_dependencies_summary.blocked_by`
- Rank divergences by what a player meets: first what stops a level being finished, then what a
  player can see or hear, then position drift. Fix in that order.
- "Unchanged or better" in an acceptance criterion means: compared leg by leg with the baseline build,
  not compared with all-green.

## 3. Compare only same-build A/B

Round 439 reported an improvement from 10 preflight failures to 1. Round 440 retracted it: the "before"
numbers came from a different tree, and the change did nothing.

Before you report a before/after table:

1. Build the baseline in a separate worktree at your branch point
   (`git worktree add <dir> <commit>`, then `cargo build --release` there).
2. Run the same scripts against both binaries in the same session.
3. Name both commits in the table header.

Never reuse a number from an earlier round as the "before" column.

## 4. Transliterate the collision chain as one unit

`src/collide.rs` is not yet one Rust function per engine function. `sweep_rooms`, `move_body`,
`doorway_advance`, `sector_after_lift` and `resting_floor` are port inventions that each cover part of
several engine functions. Rounds 436 and 437 changed one piece at a time (the `t = 0` crossing, the
containment restore, the advance distance). Each variant moved the failures to other legs.

For issue #31 (which replaces #26 and #27):

- Port the chain as one unit, in call order, and replace the port inventions:
  `sithCollision_MoveThing` (`0x4e7950`) -> `sithCollision_SearchForSurfaceCollisions` (`0x4e86d0`)
  -> `sithIntersect_IsSphereInSector` (`0x507f30`), with `sithPhysics_FindFloor` (`0x4f5550`) and
  `sithThing_SetSector` (`0x4cd1e0`) as the engine calls them.
- Do not tune one leg. Legs can be red on the branch until the whole chain is in.
- Do not stack a second experiment on an unreverted first one.

Facts already read from the decomp (do not re-derive them):

- Only `MoveThing` writes the sector during movement. It calls `SetSector` when a collision entry has
  flag `0x20` (an adjoin hit). `FindFloor` never writes the sector.
- A crossing at distance 0 is engine behaviour. The `t <= 0` skip in `sweep_rooms` is a port defect.
- When containment fails, `MoveThing` restores the position and sector saved at the start of the
  current sub-step. It never searches for another sector.
- `IsSphereInSector` tests each surface that has `HAS_COLLISION` (0x4) or whose adjoin has `MOVE`
  (0x2): `dot(position - plane_point, normal) >= radius`, with a 1e-5 snap to zero. The engine's
  normals point into the sector. The port stores them pointing out and negates the dot product. The
  two are equal on all shipped levels (round 440), so the sign is not a bug.

## 5. Check each citation against the file name in `decomp/`

A decomp file name is `<address>_<function>.c`. The address in the file name is the function's address.
`sithCollision_MoveThing` is `0x4e7950`; the `0x4f4290` in round 435 is wrong.

Check each sector list you quote against the fixture file also. Round 435 says the retail sector column
for `walk2_spawn` is `22,22,23,23,22,39`. The committed fixture
`tools/parity/walktrace/walk2_spawn-walk.tsv` reads `22,22,22,39,39,39`, and the port stays in 22 for
the whole leg. Sector 23 is on neither side; it comes from an old comment in `trace-gate.ps1` about an
earlier port build. Two rounds reasoned about a "22 -> 23 crossing" that does not exist.

To see both sector columns for a leg, do not quote them from memory or from the log:

```powershell
pwsh -NoProfile -File tools/parity/trace-gate.ps1 -Only walk2_spawn
pwsh -NoProfile -File tools/parity/trace-compare.ps1 -Name walk2_spawn
```

The second command prints the per-sample table. The port's sector for each frame is column 6 of
`target/parity/trace/<leg>.tsv`.

Decomp's `ROUND(x)` does not say which rounding it uses. The engine sets the x87 rounding mode around
each conversion: `fldcw [0x860802]` (round down), `[0x860804]` (truncate), `[0x860806]` (round up), then
`fldcw [0x860800]` (the default) again. Read the bytes before the `fistp` in `JK.EXE`. The cog
`PopInt` floors (#38 follow-up); it is not round-to-nearest.

## 6. Tool traps

- `tools/parity/walk-preflight.ps1` captures the child's stderr. To see the `contain:` counts, run
  `bevi-knight.exe --walk <frames> --warp x,y,z,yaw 2>&1` directly.
- `--walk all` asserts each `WALK_OFF_THE_EDGE` row in `src/main.rs`. The rows are port measurements
  with no retail trace. A changed row needs the engine reason in its comment.
- The retail fixture for `walk9_spawn` covers only 0 to 3 s and stops before the level's walk-off. The
  port's trace continues into the fall. The z 20.7 on that leg is not a new regression.
- `--faces-at` prints the containment result (`inside true/false`) for a point. Use it before you
  write a probe.
- The trace gate passes a leg when the port travels 75% to 125% of the retail distance and at most 2
  of the 600 ms sector samples differ. Do not change these numbers.
- `walk2_spawn`, `walk3_spawn` and `walk5_spawn` are not in the trace gate's default leg set. A plain
  `trace-gate.ps1` run does not show them. Pass `-Only <leg>`.
- The trace gate prints only the count of sector samples that differ. Run `trace-compare.ps1 -Name
  <leg>` for the table.
- The trace gate and the preflight start only the port. They do not start retail.
- `rg` is not on PATH on this machine. Use the Grep tool.
- A shell can keep `CARGO_TARGET_DIR` from an earlier worktree. Then `target/release` in your worktree
  is missing or stale, and an A/B can compare a missing or identical binary. Compare the `sha256sum`
  of both binaries before you compare their output.

## 7. When the decomp does not answer the question

Write the exact question in the handoff comment and stop that line of work. Do not launch retail, and
do not select between variants by the gate result. Before you ask, read the callers and callees: the
round 435 question was answered from the decomp in round 436.
