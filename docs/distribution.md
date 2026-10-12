# Platforms and Distribution: Windows and Linux

Status: **accepted** · Release targets for 1.0: **Windows 10/11 x64** and **Linux x64**.
macOS and Linux ARM64 are not excluded by any design choice and are deferred.

## 1. Targets and minimums

| | Windows | Linux |
|---|---|---|
| Rust target | `x86_64-pc-windows-msvc` | `x86_64-unknown-linux-gnu` |
| OS minimum | Windows 10 1809+ | glibc ≥ 2.31 (Ubuntu 20.04, Debian 11, Fedora 32+, RHEL 9) |
| GPU path | DX12 → Vulkan → GL (wgpu auto-select) | Vulkan → GL 3.3 / GLES |
| No usable GPU | CPU backend (vello_cpu/tiny-skia) + present via softbuffer | same |
| Windowing | Win32 via winit; per-monitor DPI v2 | Wayland and X11 via winit (runtime-detected) |
| System fonts | DirectWrite enumeration (fontique) | fontconfig (fontique) |
| File dialogs | `rfd` (native IFileDialog) | `rfd` with the xdg-desktop-portal backend (works in Flatpak) |
| Clipboard | `arboard` | `arboard` with Wayland data-control + X11 |
| Memory pressure | `CreateMemoryResourceNotification` | PSI `/proc/pressure/memory` (fallback: `MemAvailable` check on frame) |

**Old hardware:** wgpu's DX12 and Vulkan backends cover Intel HD 500/600 series (6th–10th gen)
and GTX 10xx. If compute shaders are unavailable or the adapter is blocklisted, the app
automatically uses the CPU renderer and shows a one-time notice in the status bar.

## 2. Platform code policy

* Platform-specific code lives only in `crates/app/src/platform/{windows,linux}.rs` (memory
  pressure, title bar, file associations, crash handler) behind a small trait. Every other crate is
  platform-neutral.
* The CI matrix builds and tests **every crate on both OSes on every PR**. A change that breaks
  either platform cannot merge.

## 3. Packages

| Platform | Artifact | Tooling | Notes |
|---|---|---|---|
| Windows | `.msi` installer | `cargo-wix` (WiX v4) | per-user install by default (no admin), Start menu, file associations `.design .fig .xd .psd`, uninstall entry |
| Windows | portable `.zip` | `cargo-dist` | runs from a USB stick; settings stored next to the exe if `portable.txt` exists |
| Windows | winget manifest | `wingetcreate` in the release workflow | `winget install kuArt` |
| Linux | AppImage | `appimagetool` / `linuxdeploy` | universal, built on the oldest supported glibc |
| Linux | `.deb` | `cargo-deb` | depends on `libvulkan1 | libgl1`, `libfontconfig1`, `libxkbcommon0` |
| Linux | `.rpm` | `cargo-generate-rpm` | same deps |
| Linux | Flatpak (Flathub) | `flatpak-builder` | `org.freedesktop.Platform` runtime; portals for files; `--device=dri` |
| Both | CLI tarball/zip | `cargo-dist` | `design-editor-cli` only, no GPU deps, for CI/servers |

Desktop integration on Linux: a `.desktop` file, a shared-mime-info XML registering
`application/x-design` plus import types, and hicolor icons at 16–512 px.

## 4. Build configuration

* Release profile: `lto = "fat"`, `codegen-units = 1`, `strip = true`. `panic` stays `unwind`
  in every binary, because the import/export sandbox relies on `catch_unwind` to survive malformed
  files (architecture.md §8).
* Linux builds run in a `manylinux`-style container (Ubuntu 20.04 image), or use `cargo zigbuild
  --target x86_64-unknown-linux-gnu.2.31`, to pin the glibc floor.
* Windows: static CRT (`-C target-feature=+crt-static`) so no VC++ redistributable is needed. An
  application manifest declares per-monitor-v2 DPI awareness, long-path awareness and UTF-8 active
  code page.
* Binary size budget: GUI ≤ 25 MB stripped, CLI ≤ 12 MB. Checked in CI.

## 5. Signing and updates

* **Windows:** Authenticode-sign the exe and msi (Azure Trusted Signing, or an OV/EV certificate)
  to avoid SmartScreen warnings. Unsigned nightlies are clearly labelled.
* **Linux:** GPG-sign release checksums, and sign AppImages with an embedded signature. Flathub
  handles its own signing.
* **Updates:** the app makes no network calls by default (offline first). An opt-in "check for
  updates" reads a signed JSON manifest. Package managers (winget, apt/dnf repos, Flathub) are the
  preferred update channel.

## 6. CI / release pipeline

```
PR / push:  matrix [windows-latest, ubuntu-22.04]
            fmt · clippy -D warnings · test · build CLI · (later) render goldens on CPU backend
nightly:    + benchmarks on self-hosted reference machines A/B (Windows + Linux)
            + fuzz each parser 10 min
tag v*:     build → package (msi, zip, AppImage, deb, rpm, cli archives) → sign → checksums
            → GitHub Release draft → winget PR → Flathub PR
```

The PR workflow is `.github/workflows/ci.yml`; the release workflow is
`.github/workflows/release.yml` (M14). Running it manually (*Actions → Release → Run workflow*)
builds every package as workflow artifacts without publishing. Pushing a tag `vX.Y.Z` that
matches the `Cargo.toml` version publishes a **draft** release with all packages and
`SHA256SUMS`.

| Output | Built with | Source |
|---|---|---|
| `KuArt-X.Y.Z-x64.msi` | WiX 5 (`wix build`) | `packaging/windows/kuart.wxs` |
| `KuArt-X.Y.Z-x64-setup.exe` | Inno Setup 6 | `packaging/windows/kuart.iss` |
| `KuArt-X.Y.Z-x64.exe` (standalone) | copy of the static-CRT binary | |
| `KuArt-X.Y.Z-windows-x64-portable.zip`, `design-editor-cli-…-windows-x64.zip` | `Compress-Archive` | |
| `kuart_X.Y.Z-1_amd64.deb` | `cargo-deb` | `[package.metadata.deb]` in `crates/app/Cargo.toml` |
| `kuart-X.Y.Z-1.x86_64.rpm` | `cargo-generate-rpm` | `[package.metadata.generate-rpm]` |
| `KuArt-X.Y.Z-x86_64.AppImage` | `appimagetool` | AppDir assembled in the workflow |
| `kuart-bin-X.Y.Z-1-x86_64.pkg.tar.zst` (Arch) | `makepkg` in an `archlinux` container | `packaging/arch/PKGBUILD` (AUR-ready) |
| `KuArt-X.Y.Z-linux-x86_64.tar.gz` (bundle) | `tar` | binaries, desktop files, icons |
| `design-editor-cli-…-linux-x86_64.tar.gz` | `tar` | |
| Flatpak | `flatpak-builder` (Flathub PR, manual) | `packaging/flatpak/` |
| winget | `wingetcreate` (manual for the first release) | `packaging/winget/README.md` |

Linux binaries are built with `cargo zigbuild --target x86_64-unknown-linux-gnu.2.31` (glibc
floor 2.31); the workflow prints the newest GLIBC symbol it links. Both jobs enforce the size
budget and smoke-test the CLI (generate → PNG export → code generation).

Signing is optional and secret-driven: `WINDOWS_CERT_BASE64` and `WINDOWS_CERT_PASSWORD` (a .pfx)
sign the exes and the MSI with signtool; `GPG_PRIVATE_KEY` signs `SHA256SUMS`. Without them,
packages are unsigned. Clean-machine install smoke tests (§7) are manual for each release.

## 7. Test matrix for releases (manual smoke + automated)

| OS | GPU | Session |
|---|---|---|
| Windows 10 22H2 | Intel HD 630 | — |
| Windows 11 | GTX 1050 Ti | — |
| Ubuntu 22.04 | Intel HD 630 (Mesa) | Wayland and X11 |
| Fedora (latest) | AMD (Mesa radv) | Wayland |
| Debian 11 | llvmpipe (no GPU) | X11 → CPU backend path |
