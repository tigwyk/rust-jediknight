# Source 0.1 release checklist

State of the tree for the **source 0.1 release** (plus a native Windows setup executable):
buildable from a clean checkout, with a player-facing note and the full check record. Nothing
here has been pushed or published.

## Done and verified on this tree

- [x] **First-run defects.** `jkPrintUNIString` reads the `.uni` key, so level 1's pickup says
  "Shield Recharge" instead of `COG_00000`; audio is on by default and `--mute` wins.
- [x] **Campaign chain end to end.** `--tour` checks all 28 campaign level rows on both arms
  (25 distinct files, 23 film hops, both finales); `--tour-live` finishes and swaps all 21
  light-side levels in the running app and stops at `21ajerec.jkl`.
- [x] **Docs.** [README](../README.md) carries the honest status and open items;
  [docs/release-0.1.md](release-0.1.md) is the builder/player note;
  [docs/gates.txt](gates.txt) ends with a fresh 61-row sweep; [docs/gaps.md](gaps.md) records
  rounds 201–204; [docs/formats.md](formats.md) has the evidence for the changed behaviour.
- [x] **Build instructions.** Windows x64, Rust **1.97.1+** (Bevy 0.20's MSRV), MSVC target,
  `rustup target add i686-pc-windows-msvc` for the film helper, and the game finds that helper
  from its own checkout (`BK_SMKDEC` overrides).
- [x] **Native installer and game discovery.** `packaging/build-setup.ps1` builds one
  self-contained `dist/bevi-knight-<version>-setup.exe` from `packaging/Setup.cs` +
  `Uninstall.cs` with the in-box .NET Framework compiler (no Inno/WiX/NSIS/Squirrel). It finds
  the retail install (GOG registry, usual folders) or accepts a browsed/pasted/dragged folder,
  installs per-user to `%LOCALAPPDATA%\Programs\bevi-knight`, saves `game-path.txt`, creates
  Start Menu/desktop shortcuts and an Apps & Features entry, and carries its uninstaller.
  The game itself also resolves `--game` → `BK_GAME` → saved paths → GOG registry → common
  folders, with `--find-game`/`--set-game` for manual setup. Tested silently (explicit path and
  registry auto-discovery), through the GUI (right game pre-filled), and through install →
  `--check` → shortcut creation → uninstall, all to temporary folders.
- [x] **Known issues.** The release note's "deliberately rough or not there" section, plus the
  README's "Open for 0.1" list.
- [x] **Check record.** `--gates` 0 of 61 failed, 0 skipped (windowed) / 9 skipped
  (`BK_HEADLESS=1`); `cargo test --bins` 132 passed.
- [x] **License.** **GPL-3.0-or-later** for code, tools and setup scripts: `LICENSE` (verbatim
  GPLv3), `NOTICE` (game/trademark/screenshot carve-out and the source-for-binaries note), and
  `license = "GPL-3.0-or-later"` in `Cargo.toml`. Contributions are inbound = outbound.
- [x] **Local tag.** `v0.1.1-rc.1`, annotated, pointing at this candidate (not pushed).

## Waiting on the repository owner

- [ ] **Version name.** `Cargo.toml` still says `0.1.0`; the local `v0.1.0` tag is far
  behind (before the campaign-chain and setup work). Keep the `v0.1.1-rc.1` candidate tag, move to
  `v0.1.1`, or re-tag a fresh `v0.1.0`?
- [ ] **Publish.** `git push origin main --follow-tags` (or the chosen branch/tag) — **only on
  explicit approval**.
- [ ] **Human playthrough.** Levels 2–21 are gated but nobody has played them start to finish;
  the dark-side choice/ending screens are absent. This is the honest ceiling on "usable 0.1".

## Re-run the checks

```sh
cargo test --bins --release
cargo build --release
./target/release/bevi-knight.exe --game "<install>" --gates
BK_HEADLESS=1 ./target/release/bevi-knight.exe --game "<install>" --gates
./target/release/bevi-knight.exe --game "<install>" --tour
./target/release/bevi-knight.exe --game "<install>" --tour-live
powershell -ExecutionPolicy Bypass -File packaging\build-setup.ps1   # dist\bevi-knight-<version>-setup.exe
.\dist\bevi-knight-<version>-setup.exe /S /GAMEPATH="<install>" /DIR="$env:TEMP\bk-test" /NOSHORTCUTS
Start-Process .\dist\bevi-knight-<version>-setup.exe   # the wizard, with the game pre-filled
```
