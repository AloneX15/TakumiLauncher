# TakumiLauncher - Development Setup

## System Requirements

### Windows 10/11

1. **Rust** (1.70+)
   - Download: https://rustup.rs/
   - Run installer and follow prompts
   - Verify: `rustc --version` && `cargo --version`

2. **Node.js** (18+)
   - Download: https://nodejs.org/
   - Install LTS version
   - Verify: `node --version` && `npm --version`

3. **pnpm** (8+)
   - Install: `npm install -g pnpm`
   - Verify: `pnpm --version`

4. **Visual Studio Build Tools** (for native modules)
   - Download: https://visualstudio.microsoft.com/downloads/
   - Select "Desktop development with C++"
   - Or install full Visual Studio Community

### Linux (Ubuntu/Debian)

```bash
# Install Rust
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

# Install Node.js
curl -fsSL https://deb.nodesource.com/setup_18.x | sudo -E bash -
sudo apt-get install -y nodejs

# Install pnpm
npm install -g pnpm

# Install development dependencies
sudo apt-get install -y \
  libwebkit2gtk-4.0-dev \
  libssl-dev \
  libgtk-3-dev \
  libayatana-appindicator3-dev
```

### macOS

```bash
# Install Rust
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

# Install Node.js (via Homebrew)
brew install node

# Install pnpm
npm install -g pnpm
```

## Initial Setup

```bash
# Navigate to project
cd takumi-launcher

# Install all dependencies
cargo build --workspace
pnpm install

# Verify setup
cargo check --workspace
pnpm list
```

## Development Commands

### Using Just (recommended)

```bash
just setup       # Full setup
just dev         # Start all dev servers
just test        # Run all tests
just fmt         # Format code
just lint        # Check code
just api         # Start backend API
just launcher    # Start desktop launcher
just admin       # Start admin panel
```

### Manual Commands

#### Backend API

```bash
cd apps/api
cargo run
# Server will start on http://127.0.0.1:3001
```

#### Desktop Launcher (requires Tauri setup)

```bash
# First time setup (builds Tauri binaries)
cd apps/launcher
cargo tauri dev
```

#### Admin Panel

```bash
cd apps/admin-panel
pnpm dev
# Opens on http://localhost:5173
```

#### Run Tests

```bash
# Rust tests
cargo test --workspace --lib

# Node tests
pnpm test
```

#### Format Code

```bash
# Rust
cargo fmt --all

# Node/TypeScript
pnpm format
```

#### Lint Code

```bash
# Rust
cargo clippy --workspace

# Node/TypeScript
pnpm lint
```

## Tauri Setup (for Desktop App)

Tauri has additional requirements:

### Windows
- Visual Studio Build Tools with C++ support (see above)

### Linux
```bash
# Already included in development dependencies above
```

### macOS
```bash
# Install Xcode command line tools
xcode-select --install
```

## Environment Variables

Create `.env` files in respective directories:

### apps/api/.env

```env
RUST_LOG=info
DATABASE_URL=postgresql://user:password@localhost/takumi_launcher
JWT_SECRET=your-secret-key
MICROSOFT_CLIENT_ID=your-client-id
MICROSOFT_CLIENT_SECRET=your-client-secret
```

### apps/launcher/.env

```env
VITE_API_URL=http://localhost:3001
VITE_ENVIRONMENT=development
```

## Architecture Overview

```
takumi-launcher/
├── apps/
│   ├── launcher/      # Desktop app (Tauri + SvelteKit)
│   ├── api/           # Backend API (Rust)
│   └── admin-panel/   # Admin UI (SvelteKit)
├── packages/          # Shared libraries
├── docs/              # Documentation
└── tests/             # Tests
```

## Debugging

### VSCode Extensions (Recommended)

- **rust-analyzer** - Rust support
- **Tauri** - Tauri development
- **Svelte for VS Code** - Svelte support
- **TypeScript Vue Plugin (Volar)** - TypeScript support

### Running with Debug Logs

```bash
# Set Rust log level
$env:RUST_LOG = "debug"

# Then run
cargo run
```

### Browser DevTools

For SvelteKit frontend:
- Press `F12` or `Ctrl+Shift+I` to open DevTools
- Use Network, Console, and Application tabs for debugging

## Troubleshooting

### "cargo not found"
- Restart your terminal after installing Rust
- Verify installation: `rustc --version`

### "Module not found" in Node
- Run: `pnpm install`
- Clear cache: `pnpm store prune`

### Tauri build fails on Linux
- Ensure all development dependencies are installed (see Linux setup above)

### WebKit errors on Windows
- Install Visual Studio Build Tools (C++ desktop development)
- Update Windows 10/11

## Testing

### Unit Tests

```bash
cargo test --workspace --lib
```

### Integration Tests

```bash
cargo test --workspace --test '*'
```

### End-to-End Tests

```bash
# To be implemented in Phase 11
pnpm test:e2e
```

## Performance Profiling

### Rust

```bash
# Build with profiling
cargo build --release

# Run with perf (Linux)
perf record ./target/release/app
perf report
```

### Frontend

- DevTools → Performance tab
- Record interactions
- Analyze frame rates and layout thrashing

## Building for Release

```bash
# Build all packages in release mode
cargo build --workspace --release

# Build desktop app for distribution
cd apps/launcher
cargo tauri build

# Outputs in:
# - target/release/ (API binary)
# - apps/launcher/src-tauri/target/release/bundle/
```

## Next Steps

1. Complete environment setup (Rust + Node)
2. Run: `just setup`
3. Run: `just test` to verify everything works
4. Start developing!

---

For questions or issues, see [CONTRIBUTING.md](../CONTRIBUTING.md)
