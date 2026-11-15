# TakumiLauncher

Premium Minecraft Launcher built for **performance**, **security**, and **scalability**.

```
TakumiLabs
    ↓
TakumiStudios
    ↓
TakumiLauncher
```

## ✅ Status

The desktop launcher is functional:

- Instances with their own game directory, memory, Java, JVM arguments and window size
- Vanilla (every version in Mojang's manifest, including legacy ones), **Fabric**, **Quilt**, **Forge** (legacy and modern installers) and **NeoForge**
- Mods, resource packs and shaders from **Modrinth**: search, install with dependencies, enable/disable, update and remove
- Modrinth modpacks (`.mrpack`), from the built-in browser or a local file
- Duplicate instances with their worlds and mods
- Parallel, SHA-1 verified downloads of client, libraries and assets with progress
- Automatic Java: the exact runtime each version needs is installed from Mojang's distribution
- Microsoft accounts (device code flow, refresh token kept in the OS credential store), which take priority, and one
  offline profile that is always the Takumi account's username (with the vanilla offline-mode UUID); players cannot
  make up offline names (`apps/launcher/src-tauri/src/accounts.rs`)
- Live game console, stop/cancel, per-instance folder access
- Mandatory **Takumi account** sign-in, a news tab and a new-version notice, all served by the
  [TakumiWeb](https://github.com/AloneX15/TakumiWeb) website (see below)

Not implemented yet: CurseForge modpacks (their API requires a key), launcher self-update, code signing, the content
API (`apps/api` only serves health/info), Microsoft sign-in on the start screen (the button is a placeholder), and the
TakumiLabs manifest / differential update pipeline (`manifest-system`, `update-engine` only define types).

## 🌐 Takumi account and website

The launcher is tied to **takumilabs.org**, whose code lives in
[AloneX15/TakumiWeb](https://github.com/AloneX15/TakumiWeb). The full contract is in
[TakumiWeb/docs/launcher-integration.md](https://github.com/AloneX15/TakumiWeb/blob/main/docs/launcher-integration.md).

- **Sign-in**: the launcher opens on a mandatory sign-in screen. "Regístrate" opens `/registro` on the website,
  where accounts are created. Sign-in uses `POST /api/launcher/login`; the token is kept in the OS credential store
  and checked on every start with `GET /api/launcher/me`. Sessions closed on the website (by the user or an admin, a
  password change or a suspension) send the launcher back to the sign-in screen. When the server cannot be reached,
  the cached profile is kept so the launcher works offline. Code: `apps/launcher/src-tauri/src/session.rs`.
- **News**: the *Noticias* tab lists the posts published on the website (`GET /api/news`).
- **Updates**: on start the launcher asks `GET /api/releases/latest` and shows a "Nueva versión" notice when the
  website has a newer version.
- **Server URL**: development builds use `http://localhost:5173`, the `pnpm dev` server of TakumiWeb. Set
  `TAKUMI_WEB_URL` when starting the launcher to use another server. Release builds get the public URL baked in at
  compile time (`TAKUMI_WEB_URL` in `release.yml`, default `https://takumilabs.org`). Only `https` URLs are used
  (plain `http` only to localhost), since passwords and the session token go there.

### Publishing a release

`.github/workflows/release.yml` runs on `v*` tags: it builds the Windows installer, creates the GitHub release (the
tag message becomes the notes; tags with `-` are pre-releases and go to the beta channel) and registers it on the
website so it appears in Downloads and triggers the update notice.

```bash
# bump the version in Cargo.toml ([workspace.package]) and apps/launcher/src-tauri/tauri.conf.json first
git tag -a v0.2.0 -m "- Novedad 1
- Corrección 2"
git push origin v0.2.0
```

Repository secrets needed for the website step: `TAKUMI_WEB_URL` (`https://takumilabs.org`) and
`TAKUMI_RELEASE_TOKEN` (same value as `RELEASE_API_TOKEN` on the web server). Without them only the GitHub release
is created.

The job runs in the GitHub environment `release`: add required reviewers to it (*Settings → Environments*) so a pushed
tag waits for approval before anything is published. Every action is pinned to a commit SHA. A published version
keeps its installer on the website (the API answers `409` to a different file), so a rebuild needs a new version.

## 🛡️ Security

- **Downloads**: every file from Mojang, loaders, Modrinth and modpacks is written to a `.part` file and only kept
  when its SHA-1 matches; Forge/NeoForge installers are never run without the checksum Maven publishes. Redirects
  from https to http are refused. Remote paths are confined to their folder (no `..`, absolute paths, drive letters,
  Windows device names like `CON` or names ending in a dot or space), and Java runtime links must stay inside it.
- **Repair**: *Instancia → Ajustes → Reparar archivos* re-hashes the client, libraries, assets and Java (a normal
  launch only compares sizes, to stay fast) and downloads again anything corrupted or modified. On a full Fabric
  instance it takes about 50 s. Checked with `TAKUMI_BENCH_INSTANCE="Fabric" cargo test -p takumi-launcher
  bench_repair -- --ignored --nocapture`, which corrupts one library, checks a normal launch keeps it and repair
  restores it.
- **Web view**: no remote content is rendered as HTML, the CSP only allows the app's own code, and the browser can
  only be opened on `https` links (or `http` to this machine).
- **Secrets**: the Takumi token and Microsoft refresh tokens live in the OS credential store; the Minecraft token is
  masked in the game console and never printed in logs.

## ⚡ Performance

**Launcher** (Ryzen 7 5800X, Windows 11, release build):

- Start-up to a usable window: about 0.5 s, most of it WebView2. It does not depend on the Takumi server: the stored
  session is shown at once and checked in the background (`session-changed` / `session-ended` events). Before that
  change start-up took 2.9 s with the server down and 15.5 s with it unreachable.
- Memory: about 32 MB for the launcher process, about 360 MB including the WebView2 processes.
- "Jugar" to the Java process starting, for an installed instance: about 0.2 s (0.6 s the first time).

**Game.** With *Ajustes → Optimizar el juego automáticamente* (on by default) the launcher starts the game with:

- `-Xms` at half the maximum heap (at least 1 GiB), so the heap is not grown through a series of early collections
- the G1 settings the official launcher uses (`G1NewSizePercent=20`, `G1ReservePercent=20`, `MaxGCPauseMillis=50`,
  region size scaled to the heap)
- above-normal process priority on Windows

Arguments set by the user come last and win; choosing a collector (`-XX:+Use…GC`) or `-Xms` turns the matching part
off. Measured booting Fabric 26.3 to the menu with a 4 GiB heap: garbage-collection pauses went from 38 (230 ms in
total) to 13 (102 ms); time to the menu was unchanged at about 10.3 s and the longest single pause rose from 19 ms
to 32 ms. Frame rate in a world was not measured.

*Contenido → Optimizar rendimiento* installs the client performance mods the instance is missing (Sodium, Lithium,
FerriteCore, Entity Culling, ImmediatelyFast, plus required dependencies) when they have a build for its loader and
game version. Mods already present are recognised by file hash, so modpacks and hand-added files are not duplicated.

Benchmarks live in `apps/launcher/src-tauri/src/bench.rs` and use the real data directory:

```bash
cargo test -p takumi-launcher bench_prepare -- --ignored --nocapture        # "Jugar" -> Java, no game window
TAKUMI_BENCH_INSTANCE="Fabric" cargo test -p takumi-launcher bench_boot -- --ignored --nocapture   # boots the game
cargo test -p takumi-launcher bench_optimize_plan -- --ignored --nocapture  # what the optimizer would install
```

## 🎯 Quick Start

### Prerequisites

- **Rust** 1.70+ ([install](https://rustup.rs/))
- **Node.js** 18+ ([install](https://nodejs.org/))
- **pnpm** 8+ (install with `npm install -g pnpm`)

### Installation

```bash
# Clone or enter the workspace
cd takumi-launcher

# Install dependencies
cargo build --workspace
pnpm install
```

### Development

```bash
# Start the desktop launcher in dev mode (needs `cargo install tauri-cli --version "^1"`)
cd apps/launcher
cargo tauri dev

# Or start the backend API
cd apps/api
cargo run
```

Launcher data (instances, libraries, assets, Java runtimes) lives in the OS app-data
directory, e.g. `%APPDATA%com.takumilabs.launcher` on Windows.

### Microsoft sign-in

Microsoft requires every launcher to use its own Azure application:

1. Register an app in the [Azure Portal](https://portal.azure.com/#view/Microsoft_AAD_RegisteredApps/ApplicationsListBlade)
   with account type **Personal Microsoft accounts only**.
2. Under *Authentication*, enable **Allow public client flows**.
3. Request access to the Minecraft API for that app id from Mojang
   (<https://aka.ms/mce-reviewappid>); new app ids are rejected until approved.
4. Provide the application (client) id in **Ajustes → Inicio de sesión con Microsoft**, or via
   the `TAKUMI_MS_CLIENT_ID` environment variable (also read at build time to bake it into a release).

## 📁 Project Structure

```
takumi-launcher/
├── apps/
│   ├── launcher/          # Desktop app: Svelte UI in src/, Rust backend in src-tauri/
│   ├── api/               # Backend API (Rust + Axum)
│   └── admin-panel/       # Admin UI (SvelteKit)
├── packages/
│   ├── minecraft-resolver/    # Minecraft version resolution
│   ├── download-manager/      # Download engine
│   ├── auth-system/           # Authentication (Microsoft + TakumiLabs)
│   ├── manifest-system/       # Manifest handling
│   ├── update-engine/         # Update & delta logic
│   ├── instance-manager/      # Instance management
│   ├── types/                 # Shared types
│   ├── shared-ui/             # UI components
│   └── telemetry/             # Logging & telemetry
├── infrastructure/        # Docker, K8s, Terraform
├── docs/                  # Documentation
└── tests/                 # Integration & E2E tests
```

## 🏗️ Architecture

See [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) for the complete architecture design.

## 🔐 Security

See [docs/SECURITY.md](docs/SECURITY.md) for security model and threat analysis.

## 🧪 Testing

```bash
# Run all tests
cargo test --workspace
pnpm test

# Run specific test
cargo test -p minecraft-resolver

# End-to-end: really installs and boots the game with an offline profile
# (downloads several hundred MB and opens a game window)
# TAKUMI_E2E_LOADER: fabric | quilt | forge | neoforge (omit for vanilla)
TAKUMI_E2E_VERSION=1.21.1 TAKUMI_E2E_LOADER=fabric cargo test -p takumi-launcher e2e_install -- --ignored --nocapture

# Content management and modpack import against the real Modrinth API (no game window)
cargo test -p takumi-launcher e2e_content -- --ignored --nocapture
```

## 🚀 Building for Release

```bash
# Build all packages in release mode
cargo build --workspace --release

# Build the desktop app for distribution
cd apps/launcher
cargo tauri build
```

## 📚 Documentation

- [UI experience](docs/UI_EXPERIENCE.md) - Visual improvements, motion, sound preferences and validation (Spanish)
- [Architecture](docs/ARCHITECTURE.md) - System design and components
- [API](docs/API.md) - Backend API specification
- [Manifest System](docs/MANIFEST.md) - Instance manifest format
- [Development Guide](docs/DEVELOPMENT.md) - Contributing guidelines
- [Deployment](docs/DEPLOYMENT.md) - Production deployment

## 🤝 Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md) for guidelines.

## 📄 License

MIT License - See LICENSE for details

---

**TakumiLauncher — Minecraft launcher built for performance.**
