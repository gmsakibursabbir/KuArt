# winget manifests

`release.yml` submits a winget PR with `wingetcreate` when the `WINGET_TOKEN` secret is set
(a GitHub token with `public_repo` scope for the fork used by wingetcreate). Package id:
`KuArt.KuArt`. To submit by hand for a release:

```bash
wingetcreate new https://github.com/gmsakibursabbir/KuArt/releases/download/vX.Y.Z/KuArt-X.Y.Z-x64.msi
```
