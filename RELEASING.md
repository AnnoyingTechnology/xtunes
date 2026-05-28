# Releasing Sustain

The `./release` helper at the repo root cuts a release end-to-end:
bumps the version, runs the full CI gate, edits the manifests, commits,
tags, and pushes. The `Release` GitHub workflow then builds the .deb
(amd64 + arm64) and Flatpak bundle and attaches them to the GitHub
release.

## Usage

```sh
./release <bump> <track> [--dry-run] [--skip-gate]
```

| argument        | values                       | meaning                              |
| --------------- | ---------------------------- | ------------------------------------ |
| `<bump>`        | `patch`, `minor`, `major`    | size of the change since last stable |
| `<track>`       | `alpha`, `beta`, `stable`    | pre-release track to publish on      |
| `--dry-run`     | flag                         | apply edits, show the diff, revert   |
| `--skip-gate`   | flag                         | skip `fmt`/`clippy`/`test`/`doc`     |

## What the script does

1. Reads the current version from `[workspace.package].version` in the
   root `Cargo.toml`.
2. Computes the new version from `<bump>` + `<track>` (rules below).
3. Refuses to run if any of the following are true:
   - current branch is not `main`
   - working tree is not clean
   - local `main` is not in sync with `origin/main`
   - the target tag `vX.Y.Z[-track.N]` already exists locally or on origin
4. Runs the full CI gate (unless `--skip-gate`):
   - `cargo fmt --all -- --check`
   - `cargo clippy --workspace --all-targets --locked -- -D warnings`
   - `cargo test --workspace --locked --no-fail-fast`
   - `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --locked --no-deps --document-private-items`
5. Asks for confirmation, then:
   - rewrites `[workspace.package].version` in `Cargo.toml`
   - prepends a `<release version=… date=… type=…/>` entry inside
     `<releases>` in `data/io.github.open_sustain.sustain.metainfo.xml`
     (keeping previous entries as release history)
   - refreshes `Cargo.lock`
6. Shows the staged diff and asks for a final confirmation.
7. Commits as `Release: bump workspace to X.Y.Z`, tags `vX.Y.Z`, and
   pushes the commit and then the tag to `origin`.
8. Prints the URL of the running Release workflow.

The Release workflow handles the rest: build .deb (amd64 + arm64),
verify install on Ubuntu 25.10 + 26.04, build Flatpak bundle, create
the GitHub release with auto-generated commit-based notes, and attach
all artifacts.

## Version computation

`<bump>` says how much of the X.Y.Z components to advance. `<track>`
picks the pre-release suffix. A pre-release version "belongs to" an
X.Y.Z that has not shipped stable yet; `patch` continues that cycle,
while `minor` / `major` start a new one.

From a pre-release (`0.1.0-alpha.2`):

| `<bump>` | `<track>` | result            | meaning                                    |
| -------- | --------- | ----------------- | ------------------------------------------ |
| patch    | alpha     | `0.1.0-alpha.3`   | next alpha iteration on the same X.Y.Z     |
| patch    | beta      | `0.1.0-beta.1`    | promote alpha → beta on the same X.Y.Z     |
| patch    | stable    | `0.1.0`           | finalize the pre-release as the stable     |
| minor    | alpha     | `0.2.0-alpha.1`   | start the next minor's alpha cycle         |
| minor    | stable    | `0.2.0`           | ship a new minor directly as stable        |
| major    | alpha     | `1.0.0-alpha.1`   | start the next major's alpha cycle         |

From a stable (`0.1.0`):

| `<bump>` | `<track>` | result            |
| -------- | --------- | ----------------- |
| patch    | alpha     | `0.1.1-alpha.1`   |
| patch    | beta      | `0.1.1-beta.1`    |
| patch    | stable    | `0.1.1`           |
| minor    | alpha     | `0.2.0-alpha.1`   |
| minor    | stable    | `0.2.0`           |
| major    | stable    | `1.0.0`           |

Refused combinations:

- `patch alpha` from `0.Y.Z-beta.N` — cannot move from beta back to
  alpha on the same X.Y.Z. Pick `patch beta`, `patch stable`, or bump
  the minor/major.

## Examples

Iterate the alpha counter (most common during pre-release):

```sh
./release patch alpha
```

Promote the current alpha to beta:

```sh
./release patch beta
```

Ship the current pre-release as the final stable:

```sh
./release patch stable
```

Start a new minor on alpha after the previous minor shipped stable:

```sh
./release minor alpha
```

Preview a release without touching git:

```sh
./release patch alpha --dry-run
```

## Recovering from a botched release

If the script fails after committing but before pushing — or after
pushing the commit but before pushing the tag — the local state is
recoverable:

- **Commit on `main` but no tag pushed yet:** delete the local tag
  with `git tag -d vX.Y.Z`, undo the commit with `git reset --soft
  HEAD~1`, fix whatever was wrong, re-run `./release`. Do not push if
  you intend to discard the commit.
- **Commit pushed, tag not pushed:** the commit is on origin; push the
  tag manually (`git push origin vX.Y.Z`) once you are happy with the
  state. The Release workflow only runs once the tag is pushed.
- **Tag pushed but workflow failed:** fix the underlying problem in a
  new commit (do not amend the release commit), then either re-run the
  failed workflow from the Actions tab, or cut the next release.
  *Never* force-push or delete a published tag — the GitHub release
  and any downstream `.deb` install would point at a now-missing SHA.

## When the script can be skipped

Manual releases (without `./release`) are fine when you specifically
want to:

- Re-run the Release workflow on an existing tag without bumping the
  version (use the `workflow_dispatch` trigger from the Actions UI).
- Cut a one-off release from a non-`main` branch (the script refuses
  this on purpose).

In every other case, prefer the script. It enforces the guards that
make `main` releasable from any clean checkout.
