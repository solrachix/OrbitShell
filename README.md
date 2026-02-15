<p align="center">
  <img src="https://github.com/solrachix/OrbitShell/blob/main/assets/logomarca.png?raw=true" alt="OrbitShell" width="240" />
</p>

# OrbitShell

A modern, block‑based terminal UI inspired by Warp, built with Rust and GPUI.

OrbitShell focuses on:
- **Block output** (command → output sections)
- **Fast search** in the sidebar (Explorer / Search / Git)
- **Workspace‑style UI** with tabs, welcome view, and settings panel
- **Local input editor** for a Warp‑like experience

> This project is open‑source and evolving. Contributions are welcome.

---

## Why OrbitShell?

- **Blazing fast** UI and search thanks to Rust + GPUI
- **Low memory usage** and snappy rendering
- **Responsive** even on large codebases

---

## Features

- Block rendering for terminal output
- Sidebar with **Explorer**, **Search**, and **Git** views
- Search across files with incremental results
- Welcome tab with recent projects
- Settings tab (Account, Code, Appearance, Keyboard Shortcuts, Referrals, MCP servers, Privacy, About)
- Windows installer via NSIS

---

## Getting Started

### Requirements

- **Rust** (latest stable)
- **Windows SDK** (for `fxc.exe`, required by `gpui`)
- **NSIS** (optional, to build the Windows installer)

### Run (dev)

```bash
cargo run
```

### Build (release)

```bash
cargo build --release
```

If you hit `Failed to find fxc.exe`, add the Windows SDK `fxc.exe` to PATH:

```powershell
$env:PATH="C:\Program Files (x86)\Windows Kits\10\bin\10.0.22621.0\x64;$env:PATH"
cargo build --release
```

The binary will be at:
```
target\release\orbitshell.exe
```

---

## Windows Installer (NSIS)

Generate the installer:

```powershell
makensis installer\windows\orbitshell.nsi
```

Output:
```
installer\windows\OrbitShell-Setup.exe
```

---

## Linux

Build on Linux (or WSL):

```bash
cargo build --release
```

To install locally:

```bash
bash installer/linux/install.sh
```

This installs the binary to `~/.local/bin/orbitshell` and adds a `.desktop` entry.

---

## Configuration

Rules file:
```
orbitshell_rules.json
```

This controls skip directories/files and search limits for the sidebar.

---

## Contributing

Issues and PRs are welcome. If you plan a larger change, open an issue first so we can align on direction.

---

## License

TBD
