## How to port (read first)

The decompiled engine in `decomp/` (index: `decomp/INDEX.txt`) is the spec. To make a mechanic match
the original game, transliterate the named engine function into Rust: same constants, same branches,
same order, its address in the doc comment.

- **Do not launch retail `JK.EXE` and do not run `tools/parity/retail-capture.ps1`.** Only the user
  starts a retail capture. The committed fixtures under `tools/parity/` are frozen regression data.
- Do not fit a constant to a capture. Read it from `decomp/` or the shipped image.
- If the decomp is ambiguous, write the exact question down for the user instead of measuring retail.

The full rules are in `docs/porting-roadmap.md` (Principles, "How to port one thing", "Rules for the
next agent or contributor").

## Agent skills

### Issue tracker

Issues live in this repo's GitHub Issues. See `docs/agents/issue-tracker.md`.

### Triage labels

The five canonical triage labels are used unchanged. See `docs/agents/triage-labels.md`.

### Domain docs

Single-context: `GLOSSARY.md` and `docs/adr/` at the repo root. See `docs/agents/domain.md`.
