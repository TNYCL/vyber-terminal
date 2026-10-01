# Releasing Vyber

The five supported build targets are Windows x64, macOS ARM64/Intel and Linux x64/ARM64.
Native build/test success is separate from real desktop acceptance. The first release is a
preview: Windows binaries are unsigned and macOS bundles are ad-hoc-signed, not notarized.
Windows x64 uses a static CRT. Packaging checks the PE import table and rejects a separate
Visual C++ runtime DLL dependency. System DLLs remain listed in `runtime-libraries.txt`.

## Workflows

- `ci.yml`: PR/main/manual checks, plus a reusable workflow for releases. All five targets
  run Clippy, tests, an optimized build and packaging. `ready` requires every job to succeed.
- `security.yml`: dependency advisories, licenses and sources; called by CI for every PR
  and release, also weekly and manually.
- `release.yml`: an existing version tag selects a main-branch commit, runs the same CI,
  validates all five artifacts, attests them, then creates a draft. It never publishes.
- `.github/dependabot.yml`: weekly Cargo and SHA-pinned Action update PRs.
- `.github/release.yml`: categories for GitHub's generated changelog.

No long-lived PAT or signing secret is required. PR jobs have read-only access.
Release write/OIDC/attestation permissions are restricted to the draft assembly job.
Native caches are keyed by OS, target, toolchain, lockfile and build configuration. GitHub's
branch scoping keeps PR caches separate from main and release tags; only main and same-repo
PRs save caches. Releases can restore trusted main caches and still rerun every check.
Keep the included cache capacity at its default; eviction or a cache miss only costs build time.

## Cut a version

1. Update `Cargo.toml` and the root package in `Cargo.lock` through Cargo. Use an alpha/beta/rc
   version until the corresponding desktop acceptance is complete. The tag must exactly match.
2. Merge the version change through a PR after `CI / ready` passes.
3. Create and push the version tag on the merged commit, for example:

   ```sh
   git tag -a v0.1.0-alpha.1 -m "Vyber 0.1.0-alpha.1"
   git push origin v0.1.0-alpha.1
   ```

4. Wait for Release. The build must contain five archives, a manifest, SHA256SUMS and build
   provenance. Each archive includes licenses, a dependency inventory and an SPDX SBOM
   describing Cargo's resolved normal/build graph for that target.
5. Download each relevant package and complete the draft's desktop acceptance list. Record
   tested OS versions; do not infer macOS 12 compatibility from the deployment target or
   all-Linux compatibility from an Ubuntu build. Keep Linux ARM64 experimental until tested.
6. Check release notes, tag, commit and all artifacts, then publish the draft in GitHub.

The Release workflow can also be manually dispatched with an existing tag. A published
release is never overwritten. A draft retry must match its original source commit.

## Verify downloads

```sh
sha256sum -c SHA256SUMS.txt
gh attestation verify <downloaded-package> --repo TNYCL/vyber-terminal
```

On Windows use `Get-FileHash -Algorithm SHA256` and compare with SHA256SUMS.txt.
Build provenance does not replace Authenticode or Apple's Developer ID notarization.

## Local packages

Python 3.11+ is required for release tooling. Rust is pinned in `rust-toolchain.toml`.

```powershell
./scripts/package-windows.ps1
```

```sh
bash scripts/package-macos.sh aarch64-apple-darwin
bash scripts/package-macos.sh x86_64-apple-darwin
bash scripts/package-linux.sh x86_64-unknown-linux-gnu
bash scripts/package-linux.sh aarch64-unknown-linux-gnu
```

Build on the matching native host. `--skip-build` (PowerShell `-SkipBuild`) only packages an
already-built explicit target. Files are written to `dist/release`. Do not ship the local
development script's machine-specific `.lnk` shortcut.

Linux build dependencies are installed by `scripts/install-linux-deps.sh` on Ubuntu.
For desktop use, install Git/XDG utilities, DejaVu/Noto fonts, a graphics driver compatible
with GPUI and a session notification service. ripgrep is optional for content search.

## Dependency maintenance exceptions

`deny.toml` blocks reported vulnerabilities, unknown sources and licenses outside its
explicit allowlist. Four maintenance-only advisories inherited through pinned GPUI/Kit
are individually accepted for this preview, with reasons next to each ID:

| Advisory | Package | Follow-up |
| --- | --- | --- |
| RUSTSEC-2024-0384 | instant | Replace on GPUI Kit upgrade |
| RUSTSEC-2024-0436 | paste | Replace on GPUI upgrade |
| RUSTSEC-2026-0206 | rustybuzz | Track upstream font stack maintenance |
| RUSTSEC-2026-0192 | ttf-parser | Track upstream font stack maintenance |

Review these at each release and on upstream upgrades. They do not suppress unrelated
future vulnerability advisories. Preserve the shipped original license files and icon
notices; the SBOM/inventory is not a legal license conclusion.

## Repository policy

After the initial successful CI, require `ready` on main, PRs for changes, and prevent
force pushes/deletion. Restrict version tag creation to maintainers, prevent tag updates
and deletion, and enable release immutability before publishing. Fix a published release
with a new patch version rather than replacing assets under an existing tag.
