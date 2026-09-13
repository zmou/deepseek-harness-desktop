<p align="center">
  <img src="tauri-app/src-tauri/icons/icon.png" width="120" alt="DeepSeek Harness Desktop logo" />
</p>

<h1 align="center">DeepSeek Harness Desktop</h1>

<p align="center">
  <strong>A desktop wrapper for
  <a href="https://github.com/deepseek-ai/deepseek-harness">DeepSeek Harness</a></strong> —
  run it on your desktop with one click. No Node.js, no pnpm, no command line required.
</p>

<p align="center">
  <samp><a href="./README.md">简体中文</a> · <strong>English</strong></samp>
</p>

<p align="center">
  <a href="LICENSE"><img alt="License: MIT" src="https://img.shields.io/badge/License-MIT-4D6BFE?style=flat-square" /></a>
  <img alt="Platform" src="https://img.shields.io/badge/Windows%20%7C%20macOS%20%7C%20Linux-black?style=flat-square" />
  <img alt="dsh version" src="https://img.shields.io/badge/dsh-0.1.2--rc.1-4D6BFE?style=flat-square" />
  <img alt="Tauri 2" src="https://img.shields.io/badge/Tauri-2-4D6BFE?style=flat-square" />
</p>

> [!IMPORTANT]
> **Unofficial project**: not affiliated with or endorsed by DeepSeek. The app icon derives from the
> official logo and is used solely to indicate compatibility with DeepSeek Harness — see [NOTICE](NOTICE).
>
> This is an **early preview** built on the rapidly evolving `@deepseek-ai/dsh@0.1.2-rc.1`.
> The upstream project states it has not been security-audited (see "Security & data" below).

<!-- Screenshots: add 2-3 real product screenshots before release (main UI, conversation, settings).
     Place them in docs/images/ and uncomment the block below:

<p align="center">
  <img src="docs/images/hero.png" width="100%" alt="DeepSeek Harness Desktop main UI" />
</p>
<table>
  <tr>
    <td><a href="docs/images/preview-1.png"><img src="docs/images/preview-1.png" alt="Conversation view" /></a></td>
    <td><a href="docs/images/preview-2.png"><img src="docs/images/preview-2.png" alt="Session list" /></a></td>
  </tr>
</table>
-->

---

## What is this

[DeepSeek Harness](https://github.com/deepseek-ai/deepseek-harness) (`dsh`) is DeepSeek AI's official
open-source agent framework: a Node process + a local HTTP server + a React web UI. This project packs
it into a desktop app:

- **1:1 replica** — the official web UI loaded in the system WebView (WebView2 / WKWebView), nothing stripped or modified
- **Zero setup** — the Node runtime is bundled in the installer, just double-click
- **Shared data** — sessions / configs / credentials are shared with the official CLI (default `~/.dsh`)
- **Clean lifecycle** — the Node child process is reaped when the window closes

## Download

<!-- TODO: add a Windows installer direct link after release (a `latest` link is recommended), e.g.:
  <a href="https://github.com/<owner>/deepseek-harness-desktop/releases/latest/download/DeepSeek-Harness-Desktop-Setup-v0.1.2-rc.1-x64.exe">
    <img src="https://img.shields.io/badge/Download-Windows_x64-4D6BFE?style=for-the-badge" alt="Download Windows x64" />
  </a>
-->

Download installers from the [Releases](../../releases) page:

| Platform | Installer filename | Status |
|---|---|---|
| Windows x64 | `DeepSeek-Harness-Desktop-Setup-v<version>-x64.exe` | ✅ available (local build) |
| macOS (Apple Silicon / Intel) | `DeepSeek-Harness-Desktop_<version>_aarch64.dmg` / `x64.dmg` | 🔧 built by CI |
| Linux x64 | `DeepSeek-Harness-Desktop_<version>_amd64.deb` / `.AppImage` | 🔧 built by CI |

> **Windows first-run**: the installer is not code-signed yet, so SmartScreen may show a warning.
> Click **"More info" → "Run anyway"** to continue.

> **macOS**: the build is ad-hoc signed. If Gatekeeper blocks the first launch, right-click the app → Open.

## What this shell adds

Beyond the official web UI, the desktop shell provides these **system-level capabilities**
(no UI modifications — the web interface is 100% official):

- **Single instance** — relaunching restores and focuses the existing window instead of spawning a second one
- **Download takeover** — session exports go through the system "Save as" dialog and remember the last directory
- **Security hardening** — one-time launch tokens are redacted everywhere in logs; `dsh web` listens on a random `127.0.0.1` port only
- **Proxy resistance** — WebView2 is forced to bypass the system proxy, so local proxies (e.g. Clash) can't hijack localhost traffic and break WebSocket
- **Process watchdog** — the window closes automatically if Node exits abnormally; stdout/stderr are continuously pumped to prevent pipe-buffer deadlocks
- **Clean UI** — the official frontend's duplicate session-export dialog is hidden (de-duplicated against the real "Save as" flow)

> These are the desktop-layer features implemented today. See the [roadmap](docs/implementation-plan.md) for planned work.

## Build

### Windows

```powershell
# Requirements: Node.js >= 22.19, Rust (msvc), Visual Studio Build Tools (C++ workload)
powershell -ExecutionPolicy Bypass -File scripts/build-win.ps1
# Pin a dsh version (syncs build-runtime / tauri.conf / Cargo.toml automatically)
powershell -ExecutionPolicy Bypass -File scripts/build-win.ps1 -DshVersion 0.1.3-alpha.2
```

### macOS

```bash
# Requirements: Node.js >= 22.19, Rust, Xcode Command Line Tools
bash scripts/build-mac.sh
```

### Runtime only (all platforms)

```bash
node scripts/build-runtime.mjs
# Output: tauri-app/resources/runtime/{node/, dsh-runtime/} (gitignored)
```

### CI

`.github/workflows/build.yml` builds a Windows / macOS / Linux matrix on `v*` tag pushes or manual
`workflow_dispatch`, uploading installers to Actions artifacts.

> **Windows deep-path escape hatch**: if the workspace path is too deep and NSIS reports a
> 260-character error, set `$env:DSH_RUNTIME_DIR = "D:\rt"` to build the runtime on a short path
> (see `docs/stage/stage-2-packaging.md` for the history).

## Development

```powershell
# dev mode uses the system node + the .stage-p0 dsh (no embedded runtime needed)
$env:DSH_BIN = "d:/Codes/deepseek-harness-desktop/.stage-p0/node_modules/@deepseek-ai/dsh/lib/bin.js"
cargo run --manifest-path tauri-app/src-tauri/Cargo.toml
```

> Run `node scripts/build-runtime.mjs` once before the first run (or keep the placeholder directory),
> otherwise `cargo run` fails because `resources/runtime` is missing (the repo ships a `.gitkeep` placeholder).

## Security & data

**Please understand the risks first**: this app runs a local AI agent on your machine that
**can execute commands and read/write files**. The upstream project
[states](https://github.com/deepseek-ai/deepseek-harness/blob/master/SAFETY.md):

> It has not been security-audited, and its sandboxing, approvals, and permission controls do not guarantee isolation.

- ⚠️ Use it only in trusted environments; avoid running it with elevated privileges
- ✅ The web service listens on a random `127.0.0.1` port only, never exposed to the LAN
- ✅ One-time launch tokens are redacted everywhere in logs; downloads reuse the official session-cookie auth
- ✅ The data directory is shared with the official CLI (default `~/.dsh`); upgrades / uninstalls do not wipe it
- ℹ️ The embedded Node runtime may trigger antivirus false positives (common for AI toolchains) — add an exclusion if needed

## Versioning

- The desktop version equals the bundled `@deepseek-ai/dsh` version (currently `0.1.2-rc.1`)
- To upgrade: change `DSH_VERSION` in `scripts/build-runtime.mjs` and keep the `tauri.conf.json` /
  `Cargo.toml` versions in sync, then rebuild (`build-win.ps1 -DshVersion` syncs all three at once)
- The `deepseek-harness/` directory is a local reference checkout of upstream source only (gitignored);
  the runtime is currently built from official npm packages — see
  `docs/github-release-sync-and-source-build-strategy.md` for the source-build roadmap

## Docs

> Most project docs are written in Chinese.

| Document | Content |
|---|---|
| [`tauri-app/README.md`](tauri-app/README.md) | Shell internals, path resolution priority, gotcha log |
| [`docs/dsh-desktop-analysis.md`](docs/dsh-desktop-analysis.md) | dsh architecture analysis and desktop packaging options |
| [`docs/implementation-plan.md`](docs/implementation-plan.md) | Overall implementation plan and phases |
| [`docs/custom-web-extension-and-upgrade-architecture.md`](docs/custom-web-extension-and-upgrade-architecture.md) | Web customization and upstream-upgrade isolation rules |
| [`docs/github-release-sync-and-source-build-strategy.md`](docs/github-release-sync-and-source-build-strategy.md) | GitHub Release sync and source-build strategy (planned) |
| [`docs/stage/`](docs/stage/) | Per-stage plans and acceptance records |

## Community

- Questions & suggestions → [Issues](../../issues)
- Contributing → [CONTRIBUTING.md](CONTRIBUTING.md)
- Changelog → [CHANGELOG.md](CHANGELOG.md)

## License

[MIT](LICENSE) © DeepSeek Harness Desktop contributors. Upstream DeepSeek Harness and its dependencies
remain subject to their respective licenses and trademark policies — see [NOTICE](NOTICE).