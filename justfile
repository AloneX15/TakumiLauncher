# TakumiLauncher Justfile
# Common development commands

set shell := ["powershell.exe", "-Command"]

# Show help
@help:
    just --list

# Setup development environment
setup:
    Write-Host "📦 Installing Rust dependencies..." -ForegroundColor Cyan
    cargo build --workspace
    Write-Host "📦 Installing Node dependencies..." -ForegroundColor Cyan
    pnpm install
    Write-Host "✅ Setup complete!" -ForegroundColor Green

# Start development mode
dev:
    Write-Host "🚀 Starting development servers..." -ForegroundColor Cyan
    pnpm dev

# Run tests
test:
    Write-Host "🧪 Running tests..." -ForegroundColor Cyan
    cargo test --workspace --lib
    pnpm test

# Format code
fmt:
    Write-Host "📐 Formatting code..." -ForegroundColor Cyan
    cargo fmt --all
    pnpm format

# Lint code
lint:
    Write-Host "🔍 Linting code..." -ForegroundColor Cyan
    cargo clippy --workspace
    pnpm lint

# Build release binaries
build-release:
    Write-Host "🔨 Building release..." -ForegroundColor Cyan
    cargo build --workspace --release
    Write-Host "✅ Release build complete!" -ForegroundColor Green

# Clean build artifacts
clean:
    Write-Host "🧹 Cleaning..." -ForegroundColor Cyan
    cargo clean
    Remove-Item -Recurse -ErrorAction SilentlyContinue node_modules
    Remove-Item -Recurse -ErrorAction SilentlyContinue dist
    Write-Host "✅ Clean complete!" -ForegroundColor Green

# Check code without building
check:
    Write-Host "✔️  Checking code..." -ForegroundColor Cyan
    cargo check --workspace
    cargo clippy --workspace --all-targets

# Run the backend API
api:
    Write-Host "🚀 Starting API..." -ForegroundColor Cyan
    cd apps/api
    cargo run

# Run the desktop launcher (Tauri dev)
launcher:
    Write-Host "🚀 Starting Launcher..." -ForegroundColor Cyan
    cd apps/launcher
    cargo tauri dev

# Run the admin panel
admin:
    Write-Host "🚀 Starting Admin Panel..." -ForegroundColor Cyan
    cd apps/admin-panel
    pnpm dev

# Generate documentation
docs:
    Write-Host "📚 Generating documentation..." -ForegroundColor Cyan
    cargo doc --workspace --no-deps --open

# Check dependencies for security issues
audit:
    Write-Host "🔐 Auditing dependencies..." -ForegroundColor Cyan
    cargo audit
    pnpm audit
