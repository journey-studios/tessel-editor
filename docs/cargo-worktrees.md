# Cargo cache across worktrees

## Installed and configured

Mr Boxington (`mbx`) 1.29.2 is installed at `~/.cargo/bin/mbx` on this Mac.
The official `aarch64-apple-darwin` archive was verified against SHA256SUMS:
`621ce235d8f241c5a3728ca03d34b9864f466f4e8b3307d8a6d7101a24a71a60`.

Use these from any checkout of this project:

```sh
./scripts/cargo build --locked
./scripts/cargo check --workspace --locked
./scripts/cargo test --workspace --locked
./scripts/cargo clippy --workspace --all-targets --locked -- -D warnings
./scripts/cargo run --locked -p tessel-editor
```

`scripts/cargo` delegates to `scripts/mbx`, which applies this project's budgets.

Update 2026-11-06: this Mac also has the global Cargo shim from `mbx setup`, so
plain `cargo ...` runs through mbx in new shells. The shim directory is prepended
to `PATH` in `~/.zshenv` and `~/.profile`, and `mbx setup` wrote the rust-analyzer
user configuration. The global budget is `gc.max_total_size = 8GiB`, the same as
the wrapper. Other machines without the shim still bypass mbx with plain Cargo,
so keep using `./scripts/cargo` here.

To install the same version on another machine, use the official release archive
or `cargo install mbx --version 1.29.2 --locked`. The wrapper resolves `mbx` from
PATH, then `~/.cargo/bin/mbx`; `TESSEL_MBX_BIN` can specify another executable.
It reports a missing tool instead of silently falling back to uncached Cargo.

## Storage and concurrency

On macOS the default shared cache is `~/Library/Caches/mbx`.
Each worktree gets its own managed target, exposed through that worktree's
`target` symlink. Equivalent eligible compilations share the action store;
worktrees do not share Cargo's target-directory lock.

The first mbx command in a checkout may adopt its existing real `target/`, moving
it under management while preserving artifacts. **Wait for any plain Cargo build
in that checkout to finish before that first adoption.** This setup did not
adopt or move the editor's target while migration was underway.

Standard paths such as `target/debug/tessel-editor` continue to work. In managed
targets, mbx gives check/clippy their own check lane automatically. Build/test/run
share the standard target lane. For two independent builds in the same checkout:

```sh
TESSEL_CARGO_LANE=build-a ./scripts/cargo build --locked
TESSEL_CARGO_LANE=test-b ./scripts/cargo test --workspace --locked
```

Those explicit lanes use `target/lanes/<name>` in the current checkout. Names
accept letters, digits, underscore and hyphen; path traversal is rejected.
Do not combine `TESSEL_CARGO_LANE` with `CARGO_TARGET_DIR`. An explicit target
setting is honored by mbx and disables its automatic target/check-lane placement.
Do not point every worktree at one shared `CARGO_TARGET_DIR`.

## Local resource policy

`.mbx.toml` reserves 2 logical CPUs and uses a shared compiler-memory budget of
10 GiB. This Mac reported 11 logical CPUs and 18 GiB RAM. Scheduler coordination
only covers compilations run through mbx; a simultaneous plain Cargo process is
outside that pool.

The project wrapper defaults to:

- `MBX_GC_MAX_TOTAL_SIZE=8GiB`: combined managed cache/build budget.
- `MBX_GC_MIN_FREE_SIZE=4GiB`: free-space safeguard.
- `MBX_DISPLAY=plain`, `MBX_SAVINGS=plain`: ordinary command/log output.

The volume had about 14 GiB free during setup. These are conservative local
values, not a quota or a promise of exact disk use; active builds may temporarily
exceed a collection budget. Existing environment overrides are preserved:

```sh
MBX_GC_MAX_TOTAL_SIZE=16GiB ./scripts/cargo build --locked
MBX_SCHEDULER_MEMORY=12GiB ./scripts/cargo build --locked
```

GC/path/machine settings are intentionally environment variables in the wrapper:
mbx rejects those keys in repository `.mbx.toml`. `share_workspace_root=false`
retains current checkout paths; do not turn it on casually, because it remaps
source paths in diagnostics/debugging. Dependency reuse already works without it.

## Debug info

The workspace `dev` profile uses `debug = "line-tables-only"` and turns debug info
off for dependencies. Backtraces keep file and line, but a debugger cannot inspect
variables. To debug workspace crates locally, override it for one build (dependencies
stay without debug info):

```sh
CARGO_PROFILE_DEV_DEBUG=true ./scripts/cargo build --locked
```

Measured on 2026-11-06 with `./scripts/cargo build --locked`: `target/debug` went
from 2.8 GiB to 2.2 GiB and `tessel-editor` from 137 MiB to 120 MiB.

## Inspection and maintenance

```sh
./scripts/mbx doctor
./scripts/mbx stats --json
./scripts/mbx explain --last
./scripts/mbx cache stats
./scripts/mbx gc --dry-run
```

Use `--dry-run` before any cleanup. No real gc, clean, broad adopt or original
artifact deletion was run during setup. Active or needed editor artifacts were
not removed. No remote cache, credentials or CI upload was configured.

`doctor` reported zero failures. It warned that APFS does not refresh access
times here, so recency/unused-unit cleanup has limits; no filesystem settings
were changed. At that time the missing-global-shim warning was expected.
APFS clone restoration was detected as supported.

## Validation — 2026-11-04

An isolated temporary Rust fixture used `serde_json` with the project's then-current
Rust toolchain, lockfile and the exact wrappers/config. Two actual Git worktrees
were built sequentially, offline, against the same empty mbx store:

| Trial | Target | Cache outcome | Wall time |
|---|---|---|---|
| Worktree A | Managed target A | Cold; 12 compilations not looked up, populated store | 4.712 s |
| Worktree B | Managed target B | 12 hits, 0 misses | 1.244 s |

The two target paths were different. Both executables ran and returned
`{"cache_fixture":true,"result":42}`. Stats reported 20,443,721 bytes in the shared
store and 20,443,721 duplicate cache bytes avoided (logical accounting).
These small-fixture timings are evidence of cache operation, **not a GPUI build
benchmark or a guaranteed speedup for the migrating editor**. First-use builds
fill a cold cache; old artifacts compiled outside mbx do not prove store warmth.

Wrapper checks passed: arguments, checkout cwd from an external working directory,
isolated lane path, traversal rejection, conflicting target rejection and budget
override. `sh -n` passed. The project doctor and GC dry run were inspected.

Evidence (temporary fixture, outside the editor repository):

- `/var/folders/h2/6xxl56j97cnfzthk1mgmpz540000gn/T/tessel-mbx-worktrees-r02o8jdf/results.json`
- `/var/folders/h2/6xxl56j97cnfzthk1mgmpz540000gn/T/tessel-mbx-worktrees-r02o8jdf/worktree-a.log`
- `/var/folders/h2/6xxl56j97cnfzthk1mgmpz540000gn/T/tessel-mbx-worktrees-r02o8jdf/worktree-b.log`

The only commit created for validation was in that temporary fixture; the editor
was not committed or pushed. Its worktrees were created solely to exercise caching.

## Official references

- [Setup](https://mr-boxington.jdx.dev/setup)
- [Managed targets](https://mr-boxington.jdx.dev/managed-targets)
- [Parallel builds](https://mr-boxington.jdx.dev/scheduling)
- [Workspace policy and resource limits](https://mr-boxington.jdx.dev/configuration)
- [Release v1.29.2](https://github.com/jdx/mr-boxington/releases/tag/v1.29.2)
