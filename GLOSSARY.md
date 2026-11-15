# Bevi Knight

A port of the Jedi Knight engine to Rust and Bevy. The decompiled engine is the specification; this
glossary gives the words the port, its gates and its issues use.

## Language

### Sources of truth

**Engine**:
The original game's code, as decompiled in `decomp/`. It is the specification.
_Avoid_: Oracle

**Retail**:
The shipped `JK.EXE` when it runs. Only the user starts it.
_Avoid_: Original, vanilla

**Port**:
This Rust program.

**Transliteration**:
A Rust function that has the same constants, branches and order as one named engine function.
_Avoid_: Reimplementation, re-derivation

**Port invention**:
A port function that has no single engine function as its source.
_Avoid_: Helper, stand-in

**Fixture**:
A committed file under `tools/parity/` that records what retail did. It is frozen regression data.
_Avoid_: Capture (a capture is the act of recording a fixture)

**Deviation**:
A known difference between the port and the engine, recorded as a row on the deviation ledger.
_Avoid_: Approximation, shortcut

**Tier**:
The level of evidence a mechanic has reached, T0 (parsed) to T4 (play parity).

### Work records

**Round**:
One numbered entry in the progress log: one unit of agent work and its measured result.

**Handoff**:
The comment on an issue that gives its current state, the rejected variants and the next single step.

**Variant**:
One experimental change that is measured and then kept or reverted.

**Same-build A/B**:
A comparison of two binaries that were built from two named commits and run through the same scripts
in one session.

### Walk checks

**Leg**:
One scripted walk from a fixed pose on one level, with a name such as `walk2_spawn`.
_Avoid_: Route (a route is the multi-leg level-1 replay), walk

**Preflight**:
The check that the port's body moves at least 0.5 units on each leg. It uses no retail data.

**Gate**:
A script that exits non-zero when the port leaves a stated band around a fixture or a decomp-derived
value. "The walk gate" is `trace-gate.ps1`.
_Avoid_: Test (a test is a Rust unit test)

**Sector sample**:
The sector that the body is in at one 600 ms point of a leg.

**Off**:
The count of a leg's sector samples that differ between the port and the fixture.

### Collision

**Sector**:
One room cell of a level; it is not always convex. A body is in one sector at a time.
_Avoid_: Room

**Adjoin**:
The opening on a surface that connects two sectors.
_Avoid_: Portal, doorway, seam, link

**Crossing**:
The change of a body's sector when its path hits an adjoin.

**Containment**:
The engine's check that a sphere is inside a sector (`sithIntersect_IsSphereInSector`).

**Restore**:
The return of a body to the position and sector saved at the start of a sub-step, after containment
fails.

**Seat**:
The placement of a body at its rest height above the floor after a spawn or a warp.

**Warp**:
The dev command that places the body at a given pose; also the port's `--warp`.

**Walk-off**:
A leg or sweep that ends with the body past a floor edge and falling.

**Pinned doorway**:
A body that stays in contact with an adjoin and crosses it again each frame.
