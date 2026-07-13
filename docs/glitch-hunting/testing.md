# Testing and visual TAS playback

## VS Code test selector

Run **Tasks: Run Test Task** from the command palette and choose
**Glitch Hunt: Run Selected Test**. VS Code presents one fixed selector:

- `all`: native automation, Aurora time, Rust tests/lint, and a real two-worker
  smoke run;
- `native`: every game-data-free C++ and Aurora test;
- `input-tape`, `game-clock`, `name-entry`, `name-entry-trace`, `rng`, or
  `aurora-time`: one native suite;
- `rust` or `rust-lint`: the control-plane tests or formatting/Clippy; and
- `worker-smoke` or `pool-smoke`: real native process protocol tests.

The same selector works without VS Code:

```powershell
.\tools\glitch-hunting\run-tests.ps1 -Test all
.\tools\glitch-hunting\run-tests.ps1 -Test input-tape
.\tools\glitch-hunting\run-tests.ps1 -Test pool-smoke
```

The runner configures `windows-clang-debug` with code mods disabled. This keeps
the automation tests usable from ordinary PowerShell rather than requiring a
Visual Studio developer shell.

## Visible TAS playback

1. Put a supported Twilight Princess disc image somewhere on your machine.
   `orig/GZ2E01/GZ2E01.iso` is the default prompt value and remains ignored by
   Git.
2. Run **Tasks: Run Task** and choose **Glitch Hunt: Play Visual TAS**.
3. Enter the DVD path and confirm or replace the DUSKTAPE path. Leaving the DVD
   prompt blank uses the last image selected through Dusklight's Browse screen.

The pre-launch task builds Dusklight and compiles
`tests/fixtures/automation/boot_start_smoke.json` into
`build/boot_start_smoke.tape`. The game then runs with its normal visible
renderer and normal pacing. Exclusive automation input begins on the first game
tick. The smoke tape waits 30 ticks, presses Start for one tick, then releases
the controller; playback hands back a neutral controller when the tape ends.

Before launching, the wrapper removes copied surrounding quotes, resolves
relative paths against the repository, and verifies that both files exist. It
prints the exact absolute paths it passes to Dusklight. A missing or stale DVD
prompt falls back to Dusklight's last valid manual selection rather than
silently opening the prelaunch screen.

This is live game playback, not video rendering of a previous run. To attach a
debugger and use breakpoints, choose **Glitch Hunt: Visual TAS Playback** under
**Run and Debug** and press F5 instead. That configuration uses the same build,
fixture, and path prompts and requires VS Code's `cppvsdbg` debugger support.

The checked launcher is also directly callable. Surrounding quotes pasted as
part of either path are accepted:

```powershell
.\tools\glitch-hunting\play-visual-tas.ps1 `
  -DvdPath 'C:\path with spaces\game.iso' `
  -TapePath 'build\boot_start_smoke.tape'
```

## Authoring a custom tape

Copy the smoke JSON fixture, add exact frames/repeats/holds, and compile it:

```powershell
cargo run --manifest-path tools/huntctl/Cargo.toml -- tape compile `
  my-test.json build/my-test.tape
```

Select `build/my-test.tape` when starting the visual TAS configuration. Inspect
the expanded controller frames before playback with:

```powershell
cargo run --manifest-path tools/huntctl/Cargo.toml -- tape inspect `
  build/my-test.tape --frames
```

Visible playback is intentionally paced. For a fast non-rendering replay of
the same tape, run:

```powershell
.\build\windows-clang-debug\dusklight.exe --headless --dvd game.iso `
  --input-tape build/my-test.tape --exit-after-tape
```

The current tape starts at process boot. Direct stage/save initialization can
still be supplied with the existing `--stage` and `--load-save` CLI options,
but those fields are not yet embedded in a portable scenario artifact.
