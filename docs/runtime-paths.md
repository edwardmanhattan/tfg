# Installed runtime paths

The release executable embeds the catalog, fleet, land, replay scenarios, and
map seed. It does not need the source tree or Rust toolchain to run.

## The executables in the bundle

Every bundle carries all three roles, side by side:

- `tfg` — the console. It is also the control host when `TFG_CONTROL_LISTEN`
  and `TFG_CONTROL_TOKEN` are set: that process owns the shared camera and
  publishes what its window renders.
- `tfg-server` — the control plane with no window. The process owns the
  camera by itself, for a machine with no display; `--listen`, `--token`,
  `--devices` and `--ephemeral` are described by `tfg-server --help`.
- `tfgctl` — the reference client. `tfgctl pair` opens a pairing window and
  prints a QR code, `tfgctl devices` lists what was paired, and `status`,
  `session`, `camera`, `focus` and `panel` drive the host. It reads
  `TFG_CONTROL_URL` and `TFG_CONTROL_TOKEN`, or `--server` and `--token`.

On macOS the console is `ARCONS.app/Contents/MacOS/tfg` and the other two sit
in `ARCONS.app/Contents/Helpers`. On Linux they are three files in the tarball
(and in `/usr/bin` in the Debian package); on Windows, three `.exe` files
beside each other.

The bundles are self-contained with respect to TFG's own resources. They are
not portable across operating systems or CPU architectures. Linux builds from
Ubuntu 24.04 expect a compatible glibc and native OpenGL/Vulkan runtime;
Windows uses its native desktop runtime. macOS bundles non-system dylibs in
`ARCONS.app/Contents/Frameworks` and rewrites their load paths, while still
using macOS system frameworks.

Writable state is selected in this order:

1. `TFG_DATA_DIR=/path/to/tfg-data`
2. `TFG_PORTABLE=1` → `<executable>/data` (on macOS, the `data` directory next to `ARCONS.app`; use only when that directory is writable)
3. Platform data directory:
   - Linux: `${XDG_DATA_HOME:-~/.local/share}/tfg`
   - macOS: `~/Library/Application Support/tfg`
   - Windows: `%LOCALAPPDATA%\\tfg`

A `.env` in the working directory is loaded before path discovery, so it
can select `TFG_DATA_DIR` or `TFG_PORTABLE`; a data-directory `.env` is loaded
immediately afterward for the remaining settings. Real environment variables
always win.

The data directory contains the local SQLite mirror, map cache, last-user
record, and session journals.

A release binary can verify its embedded resources without opening a window:

```text
tfg --check-runtime
```

The GitHub Actions matrix runs that command before packaging. The resulting
bundles include generated dependency notices plus the MapLibre Native license.
The Linux build publishes a portable tarball, while a separate Debian 13
container publishes an installable `amd64 .deb` with Debian-native dependency
versions. All bundles are attached to matching `v*` GitHub releases and are
also available as Actions artifacts for other builds.
