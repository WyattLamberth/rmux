# Infrastructure commands

Related backlog: [Nix #3](https://github.com/WyattLamberth/rmux/issues/3),
[CI #4](https://github.com/WyattLamberth/rmux/issues/4), and
[telemetry #5](https://github.com/WyattLamberth/rmux/issues/5).

Run `bash scripts/ci.sh` for workspace formatting, Clippy with warnings denied,
unit/integration/doc tests, the core dependency boundary, and five deterministic
universe seeds. Cargo build and test commands preserve `Cargo.lock` with
`--locked`. Replay a failure with:

```sh
RMUX_TEST_SEED=42 RMUX_TEST_STEPS=20000 bash scripts/test-seeds.sh
# Or run multiple decimal u64 seeds in order:
RMUX_TEST_STEPS=2000 bash scripts/test-seeds.sh 0 1 42
```

The runner prints seed and step count before each test. The universe test must
report the failing step and a sanitized deterministic trace. CI runs the five
seeds independently with 20,000 steps and `fail-fast: false`.

## Nix

```sh
nix develop --no-update-lock-file
nix build --no-update-lock-file
nix flake check --no-update-lock-file --print-build-logs
nix run --no-update-lock-file
```

Flake outputs support `x86_64-linux` and `aarch64-linux`; CI builds/checks the
native x86_64 Linux output. `Cargo.lock` supplies crate hashes. `flake.lock`
pins Nixpkgs 26.05, including the compiler and build tools. The Nixpkgs lock
entry was copied from the official Nix project's lockfile, not computed locally.
Nix has not been available for local evaluation; CI must verify the lock hash
and builds. The source includes the root binary and `crates/` workspace members;
installation asserts that `bin/rmux` exists. The source also includes benchmark
examples. Package checks run library/binary unit tests, the deterministic universe,
and doc tests; real PTY and HTTP socket integration tests run in a dedicated
GitHub job because Nix sandboxes may restrict these resources. `RMUX_TEST_SHELL`
uses the packaged Nix shell, and coreutils supplies `stty` and `sleep`.
Format, Clippy, and universe checks use the Nix compiler directly without rustup
downloads. The dev shell supplies
Rust tools, actionlint, shellcheck, and cargo-audit. A flake in a Git checkout
can only see tracked files: include the new infrastructure and workspace files
in Git before evaluating it.

## CI and version-tag artifacts

The workflow uses pinned action commits and read-only repository permissions.
Dependency advisories are checked against the current RustSec database, so
that check intentionally depends on network access and evolving advisories.
Coverage uses pinned cargo-llvm-cov 0.9.1 with LLVM tools and uploads HTML and
LCOV reports as workflow artifacts. It tests the complete workspace; no arbitrary
coverage percentage is enforced before a baseline is established. A separate
job runs the production transition benchmark in release mode and records results
and compiler/revision metadata. Hosted-runner timings have no performance
threshold. Cargo run selects the root package explicitly because it does not
support `--workspace`. Coverage tool installation uses its published lockfile.
No release or registry is published. A `v<Cargo.toml version>` tag, after all
quality jobs pass, produces a workflow artifact containing a Linux x86_64 GNU
archive and its SHA-256 checksum. Download the artifact from that workflow run.

The release binary is built on Ubuntu 24.04 with Rust 1.96.1 and targets
`x86_64-unknown-linux-gnu`; it requires a compatible Linux/glibc environment
(the builder uses glibc 2.39). It uses dynamic system libraries. The Nix package
provides a separate build with pinned dependencies and tools. Archive metadata
records the Git revision and lockfile hashes; archive timestamps and ownership
are normalized. This guarantees deterministic repackaging of identical input
files, not bit-for-bit reproducibility of the GitHub-hosted compiler environment.

```sh
python3 scripts/package-release.py --tag v0.1.0 \
  --revision "$(git rev-parse HEAD)" --target x86_64-unknown-linux-gnu \
  --binary target/x86_64-unknown-linux-gnu/release/rmux --output /tmp/rmux-dist
(cd /tmp/rmux-dist && sha256sum --check ./*.sha256)
```
