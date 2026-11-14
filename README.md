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
- Vanilla (every version in Mojang's manifest, including legacy ones), **Fabric** and **Quilt**
- Parallel, SHA-1 verified downloads of client, libraries and assets with progress
- Automatic Java: the exact runtime each version needs is installed from Mojang's distribution
- Microsoft accounts (device code flow, refresh token kept in the OS credential store) and offline profiles
- Live game console, stop/cancel, per-instance folder access

Not implemented yet: Forge / NeoForge, modpack import (Modrinth / CurseForge), the content
API (`apps/api` only serves health/info), the admin panel, and the TakumiLabs manifest /
differential update pipeline (`manifest-system`, `update-engine` only define types).

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
TAKUMI_E2E_VERSION=1.21.1 TAKUMI_E2E_LOADER=fabric   cargo test -p takumi-launcher e2e -- --ignored --nocapture
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
