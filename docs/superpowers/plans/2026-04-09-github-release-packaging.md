# GitHub Release Packaging Implementation Plan

> **For agentic workers:** REQUIRED: Use superpowers:subagent-driven-development (if subagents available) or superpowers:executing-plans to implement this plan. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build and publish installable OrbitShell release assets for Linux and Windows through GitHub Releases.

**Architecture:** Use a tag-triggered GitHub Actions workflow with three packaging lanes: native Windows installer via NSIS, Linux `.deb` via `cargo-deb`, and Linux `AppImage` via a dedicated AppImage toolchain. Keep versioning driven by the git tag and reuse the existing NSIS/script assets already present in the repository.

**Tech Stack:** GitHub Actions, Rust/Cargo, `cargo-deb`, NSIS, AppImage packaging, bash/PowerShell glue

---

## Chunk 1: Packaging Metadata

### Task 1: Add Debian packaging metadata

**Files:**
- Modify: `Cargo.toml`

- [ ] **Step 1: Add Debian package metadata**

Add package metadata for name, maintainer, description, assets, desktop integration, and icon so `cargo deb` can produce installable `.deb` files.

- [ ] **Step 2: Keep Linux desktop assets explicit**

Reference repository assets and install targets clearly instead of generating everything ad hoc in CI.

- [ ] **Step 3: Commit**

```bash
git add Cargo.toml
git commit -m "build: add deb packaging metadata"
```

### Task 2: Add Linux desktop packaging assets

**Files:**
- Create: `packaging/linux/orbitshell.desktop`
- Create: `packaging/linux/AppRun`

- [ ] **Step 1: Add the desktop entry**

Create a stable `.desktop` file using `OrbitShell` branding and the installed executable name.

- [ ] **Step 2: Add AppImage launcher stub**

Create a minimal `AppRun` entrypoint for AppImage bundles.

- [ ] **Step 3: Commit**

```bash
git add packaging/linux/orbitshell.desktop packaging/linux/AppRun
git commit -m "build: add linux desktop packaging assets"
```

## Chunk 2: Release Automation

### Task 3: Add tag-driven release workflow

**Files:**
- Create: `.github/workflows/release.yml`

- [ ] **Step 1: Build release workflow skeleton**

Trigger on version tags and set shared version environment variables derived from the tag.

- [ ] **Step 2: Add Linux x86_64 packaging job**

Build the release binary, produce `.tar.gz`, `.deb`, and `AppImage`, then upload them as workflow artifacts.

- [ ] **Step 3: Add Linux arm64 packaging job**

Build the release binary for `aarch64-unknown-linux-gnu`, produce `.tar.gz`, `.deb`, and `AppImage`, then upload them as workflow artifacts.

- [ ] **Step 4: Add Windows packaging job**

Build the release binary on `windows-latest`, compile the NSIS installer, and upload both the raw `.exe` and installer.

- [ ] **Step 5: Add GitHub Release publish job**

Create or update the GitHub Release for the tag and attach all built assets.

- [ ] **Step 6: Commit**

```bash
git add .github/workflows/release.yml
git commit -m "ci: add release packaging workflow"
```

## Chunk 3: Installer Integration

### Task 4: Make NSIS versioning tag-aware

**Files:**
- Modify: `installer/windows/orbitshell.nsi`

- [ ] **Step 1: Parameterize installer version**

Allow CI to inject the version instead of hardcoding `0.1.0`.

- [ ] **Step 2: Keep local fallback sane**

Preserve a default local version so the installer can still be built manually.

- [ ] **Step 3: Commit**

```bash
git add installer/windows/orbitshell.nsi
git commit -m "build: make windows installer version configurable"
```

## Chunk 4: Docs And Verification

### Task 5: Document the release flow

**Files:**
- Modify: `README.md`

- [ ] **Step 1: Document tagging and outputs**

Explain how to cut a release tag and which assets GitHub Releases will publish.

- [ ] **Step 2: Document local verification commands**

List the core local checks for Linux packaging and note that Windows packaging is produced in CI.

- [ ] **Step 3: Commit**

```bash
git add README.md
git commit -m "docs: document release workflow"
```

### Task 6: Verify the local release path

**Files:**
- Modify as needed from earlier tasks

- [ ] **Step 1: Run formatting**

Run: `cargo fmt`
Expected: clean formatting

- [ ] **Step 2: Run library tests**

Run: `cargo test --lib`
Expected: PASS

- [ ] **Step 3: Run Linux release build**

Run: `cargo build --release`
Expected: PASS and `target/release/orbitshell` exists

- [ ] **Step 4: Commit final cleanup**

```bash
git add .
git commit -m "build: finalize release packaging setup"
```

