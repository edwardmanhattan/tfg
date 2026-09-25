# Installed runtime paths

The release executable embeds the catalog, fleet, land, replay scenarios, and
map seed. It does not need the source tree or Rust toolchain to run.

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
The Linux build publishes both a portable tarball and an installable
`amd64 .deb`; all bundles are attached to matching `v*` GitHub releases. They
are also available as Actions artifacts for other builds.
