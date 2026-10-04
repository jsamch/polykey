# Releasing polykey

The workflow `.github/workflows/release.yml` builds, checks and publishes the release files.
Choices and their reasons are in `docs/DECISIONS.md` entry 11.

## Cut a release

1. Set `version` under `[workspace.package]` in `Cargo.toml`, update `Cargo.lock`
   (`cargo build --locked` must still pass), merge to `main` through a PR with green CI.
2. Tag the merge commit and push the tag. The tag must be `v` plus the workspace version,
   or the workflow stops at its first job.

   ```
   git tag -s v0.2.0 -m "polykey 0.2.0"
   git push origin v0.2.0
   ```
3. The workflow builds Windows, macOS and Linux in parallel. Each job runs `polykey selftest`,
   compares `polykey --help` with the snapshot, and (Linux, macOS) runs `tools/cross_check.py`.
   Nothing is uploaded if a check fails. The last job packages the files, writes the SBOM and
   `SHA256SUMS`, and creates the GitHub release with all of them.

## Dry run

Actions, "Release", "Run workflow" on any branch. It does everything except create a release,
and uploads the files as the workflow artifact `dist`. Use it after any change to the
workflow or to build flags. File names carry `-dryrun-<commit>` as the version.

## Files in a release

| File | Content |
| --- | --- |
| `polykey-<version>-windows-x86_64.exe` | portable, static CRT, console exe that detaches on double-click |
| `polykey-<version>-macos-universal.tar.gz` | arm64 and x86_64, macOS 11 or newer |
| `polykey-<version>-linux-x86_64.tar.gz` | glibc 2.35 or newer; the GUI needs a GL driver and libxkbcommon plus X11 or Wayland libraries |
| `polykey-<version>-<platform>.cdx.json` | CycloneDX 1.5 SBOM per platform (windows-x86_64, macos-aarch64, macos-x86_64, linux-x86_64): the crates that go into `polykey-app` with `gui` |
| `SHA256SUMS` | SHA-256 of every file above |

Release v0.1.0 was published under the old name, with files named `bcp-0.1.0-...` and a binary
called `bcp` (DECISIONS entry 12). From v0.2.0 on the files and the binary are named polykey.

## Verify a download

```
sha256sum --check --ignore-missing SHA256SUMS      # Linux
shasum -a 256 --check --ignore-missing SHA256SUMS  # macOS
Get-FileHash polykey-<version>-windows-x86_64.exe -Algorithm SHA256   # Windows, compare by eye
```

To check that the published binary comes from the source, check out the tag, install the pinned
toolchain from `rust-toolchain.toml`, and build with the flags in the workflow (profile settings
are in `Cargo.toml`; `RUSTFLAGS`, `SOURCE_DATE_EPOCH` and `CARGO_INCREMENTAL` are in the
workflow). On Linux use Ubuntu 22.04, and leave the target directory at its default (generated bindings embed the build output path). The unsigned binary should hash the same as the one in
the archive. See entry 11 for what can differ between machines.

The SBOM lists every crate that goes into the binary. Inspect it with any CycloneDX tool, or
for example `jq -r '.components[] | "\(.name) \(.version)"' polykey-<version>-linux-x86_64.cdx.json`. To
regenerate one, run `cargo install cargo-cyclonedx --version 0.5.9 --locked`, then
`cargo cyclonedx --manifest-path crates/polykey-app/Cargo.toml --format json --spec-version 1.5 --features gui --target x86_64-unknown-linux-gnu`.

## Signing secrets (step 7.3)

The workflow has placeholder signing steps that are skipped while these repository (or
environment) secrets do not exist. Keys live only in GitHub encrypted secrets.

| Secret | Use |
| --- | --- |
| `WINDOWS_CERT_PFX_BASE64` | Authenticode certificate, base64 of the .pfx; presence enables the step |
| `WINDOWS_CERT_PASSWORD` | password of that .pfx |
| `MACOS_CERT_P12_BASE64` | Developer ID Application certificate, base64 of the .p12; presence enables the step |
| `MACOS_CERT_PASSWORD` | password of that .p12 |
| `MACOS_SIGN_IDENTITY` | signing identity name for `codesign` |
| `APPLE_ID` | Apple ID used for `notarytool` |
| `APPLE_TEAM_ID` | Apple developer team ID |
| `APPLE_APP_PASSWORD` | app-specific password for `notarytool` |

Until 7.3 lands, set none of them. If one of the enabling secrets is set early, the placeholder
step fails on purpose so an unsigned file is never published as signed.
