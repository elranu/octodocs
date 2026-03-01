# OctoDocs Release Process

## Overview

Releases are driven by **git tags**. Pushing a tag matching `v*` to GitHub
triggers the [release workflow](.github/workflows/release.yml), which builds
all three platforms and publishes a GitHub Release with installers and binaries
attached.

---

## Step-by-step

### 1. Bump the version

Edit `desktop/crates/octodocs-app/Cargo.toml` — the `version` field **must
match** the tag you are about to push.

```toml
# desktop/crates/octodocs-app/Cargo.toml
version = "0.2.1"   # ← change this
```

The value is read at compile time via `env!("CARGO_PKG_VERSION")` and used in:
- The **status bar** display (`v0.2.1`).
- The **auto-updater** comparison against the latest GitHub release tag.

Rebuild locally to update `Cargo.lock`:

```bash
cd desktop
cargo build -p octodocs-app
```

Commit both files:

```bash
git add desktop/crates/octodocs-app/Cargo.toml desktop/Cargo.lock
git commit -m "chore: bump version to 0.2.1"
```

### 2. Create and push the tag

```bash
git tag v0.2.1
git push origin HEAD v0.2.1
```

> The branch does **not** have to be `main`. Push any branch + the tag.
> The workflow key is just `on: push: tags: - "v*"`.

### 3. Watch the CI

In the repository **Actions** tab, the **Release** workflow starts automatically.

| Job | Runner | Required? |
|-----|--------|-----------|
| Build · Linux x86_64 | `ubuntu-latest` | **Yes** (blocks release) |
| Build · macOS aarch64 | `macos-latest` | No (`continue-on-error`) |
| Build · Windows x86_64 | `windows-2022` | No (`continue-on-error`) |
| Create GitHub Release | `ubuntu-latest` | After all three |

The release is created only if the Linux build succeeds. macOS and Windows
artifacts are attached when available.

> **Note:** The Linux job runs `cargo clippy --workspace -- -D warnings` as a
> gate. Fix all warnings before tagging — a single `warning:` will fail the
> release build.

### 4. Verify the release

Once CI finishes, go to **Releases** and confirm:

- `octodocs-linux-x86_64.tar.gz` and `octodocs-linux-x86_64` are attached.
- The release notes were auto-generated from commit messages.
- (Optional) macOS `.dmg`, Windows `.exe` installer are attached if those jobs passed.

---

## What the CI builds

### Linux
- `cargo build --release -p octodocs-app`
- Packages binary + assets as `octodocs-linux-x86_64.tar.gz`
- Also provides a direct `octodocs-linux-x86_64` bare binary

### macOS (Apple Silicon)
- `cargo build --release -p octodocs-app`
- Creates `.app` bundle with `Info.plist`
- Packages as `octodocs-macos-aarch64.tar.gz` + `octodocs-macos-aarch64.dmg`

### Windows
- `cargo build --release -p octodocs-app`
- Runs `ISCC.exe` on `resources/windows/octodocs.iss` to produce
  `OctoDocs-Setup-x86_64.exe` (Inno Setup wizard installer)
- Also uploads the raw `octodocs-windows-x86_64.exe` portable binary

---

## Auto-updater integration

`updater.rs` hits `https://api.github.com/repos/elranu/octodocs/releases/latest`
and compares `tag_name` to `env!("CARGO_PKG_VERSION")`. If the latest release
tag (e.g. `v0.2.1`) is newer than the running binary, a blue **"Update
available"** banner appears in the toolbar.

Clicking **Update now**:
- **Linux/macOS**: runs `curl -fsSL install.sh | sh` in the background.
- **Windows**: downloads `OctoDocs-Setup-x86_64.exe` to `%TEMP%` and runs it
  silently (`/verysilent /update=true`), then exits.

---

## Versioning convention

```
v<MAJOR>.<MINOR>.<PATCH>
```

- **PATCH** — bug fixes, small improvements (e.g. `v0.2.1`)
- **MINOR** — new user-visible features (e.g. `v0.2.0`)
- **MAJOR** — breaking changes or major milestones

The `Cargo.toml` `version` and the git tag **must always match**. Never push a
tag without bumping `Cargo.toml` first.

---

## Checklist before tagging

- [ ] `cargo clippy --workspace -- -D warnings` passes locally
- [ ] `cargo test -p octodocs-core` passes
- [ ] `version` in `desktop/crates/octodocs-app/Cargo.toml` matches the intended tag
- [ ] `Cargo.lock` updated (`cargo build`)
- [ ] Changes committed and pushed to GitHub
- [ ] Tag pushed: `git tag vX.Y.Z && git push origin HEAD vX.Y.Z`
