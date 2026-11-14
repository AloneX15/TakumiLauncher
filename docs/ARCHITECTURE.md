# TakumiLauncher - Architecture

## Overview

TakumiLauncher is a premium Minecraft launcher built with Rust, Tauri, and SvelteKit. It provides:

- Fast, lightweight desktop application (< 200MB)
- Minecraft version management (vanilla, snapshots, old versions)
- Mod loader support (Fabric, Forge, NeoForge, Quilt)
- Modpack importing from Modrinth and CurseForge
- Instance management and versioning
- Differential updates
- Microsoft and TakumiLabs authentication
- Backend API for content distribution
- Admin panel for managing content

## Technology Stack

| Layer | Technology | Reason |
|-------|-----------|--------|
| Desktop | Tauri 1.5 + Rust | Best performance, memory, startup |
| Frontend | SvelteKit + TypeScript | Reactive, fast, beautiful animations |
| Backend API | Axum + Tokio (Rust) | High performance, type-safe |
| Database | PostgreSQL + SQLite | Reliable, scalable |
| Storage | S3-compatible | Distributed, CDN-ready |
| CDN | Cloudflare/AWS | Global fast delivery |

## Architecture Layers

### Desktop Application

```
┌─────────────────────────────────────┐
│         SvelteKit Frontend           │
│   (Dark UI + Crimson theme)          │
└────────┬────────────────────────────┘
         │ Tauri IPC
         ▼
┌─────────────────────────────────────┐
│       Tauri Runtime (Rust)          │
│  - File system access               │
│  - Process management               │
│  - System integration               │
└────────┬────────────────────────────┘
         │
         ▼
┌─────────────────────────────────────┐
│      Core Rust Libraries            │
│  - Auth System                      │
│  - Download Manager                 │
│  - Instance Manager                 │
│  - Minecraft Resolver               │
│  - Update Engine                    │
└────────────────────────────────────┘
```

### Backend API

```
┌─────────────────────────────────────┐
│        SvelteKit Admin Panel        │
└────────┬────────────────────────────┘
         │ HTTP/REST
         ▼
┌─────────────────────────────────────┐
│       Axum Web Framework            │
│  - Route handlers                   │
│  - Middleware (auth, CORS, etc)     │
│  - WebSocket support                │
└────────┬────────────────────────────┘
         │
         ▼
┌─────────────────────────────────────┐
│       Business Logic                │
│  - Authentication                   │
│  - Instance Management              │
│  - File Management                  │
│  - Manifest Generation              │
│  - Content Distribution             │
└────────┬────────────────────────────┘
         │
         ▼
┌──────────────────────────────────────────────┐
│           Data Storage                        │
│  - PostgreSQL (users, instances, etc)       │
│  - S3 (files, binaries)                      │
│  - Redis (cache, sessions)                   │
└──────────────────────────────────────────────┘
```

## Module Organization

### apps/launcher
- **Frontend**: SvelteKit + Svelte components
- **Backend**: Tauri + Rust runtime
- **Responsibilites**: 
  - UI rendering
  - Local file system management
  - Instance launching
  - Process management

### apps/api
- **Framework**: Axum + Tokio
- **Responsibilities**:
  - Authentication (Microsoft OAuth2, TakumiLabs)
  - Instance API
  - File management
  - Manifest publishing
  - Update distribution

### packages/*

Each package is a reusable Rust library:

- **auth-system**: OAuth2, JWT, token management
- **download-manager**: Parallel downloads, resume, retry
- **minecraft-resolver**: Version manifests, libraries, assets
- **manifest-system**: Manifest format, signing, validation
- **update-engine**: Differential updates, staging, rollback
- **instance-manager**: Instance lifecycle, health checks
- **types**: Shared type definitions
- **shared-ui**: Reusable UI components (Svelte)
- **telemetry**: Logging, metrics collection

## Data Flow

### Instance Launch Flow

```
User clicks "Play"
        │
        ▼
Check instance health
        │
        ▼
Verify Minecraft version exists
        │
        ├─→ If missing: Download via Download Manager
        │
        ▼
Resolve Java version
        │
        ├─→ If missing: Download Eclipse Temurin
        │
        ▼
Build launch configuration
        │
        ▼
Start Minecraft process
        │
        ▼
Monitor for completion
        │
        ▼
Check for updates in background
```

### Update Flow

```
Background check: GET /api/v1/instances/{id}/update
        │
        ▼
Manifest comparison (version + hashes)
        │
        ├─→ No update: Done
        │
        ├─→ Update available: Download in staging/
        │
        ▼
Download all changed files
        │
        ▼
Verify hashes (SHA256)
        │
        ▼
Minecraft process closed
        │
        ▼
Atomic apply:
  1. Backup current → backup/
  2. Move staging → current/
  3. Verify success
        │
        ├─→ Success: Ready
        ├─→ Failure: Restore from backup/
```

## Security Model

### Threat Mitigation

| Threat | Mitigation |
|--------|-----------|
| MITM | HTTPS + TLS 1.3, certificate pinning |
| Malicious Manifest | Digital signatures, verification |
| Compromised CDN | SHA256 hash verification |
| Modified Mod | Hash verification, optional ClamAV |
| Token Theft | Keychain/Credential Manager storage |
| Path Traversal | Path validation, no `../` |
| DLL Injection | Code signing, sandboxing |

### Implementation

- **HTTPS**: All communications encrypted
- **Manifest Signing**: RSA-2048 signatures
- **File Verification**: SHA256 hashes
- **Secure Storage**: OS Keychain (Windows/macOS/Linux)
- **Rate Limiting**: Per-user, per-endpoint
- **Audit Logging**: All admin actions logged

## Performance Targets

| Metric | Target |
|--------|--------|
| Startup | < 2 seconds |
| Idle RAM | < 150MB |
| Download Speed | Limited by network, not app |
| UI Responsiveness | 60 FPS (animations) |
| Manifest Processing | < 500ms |

## Development Workflow

```
Feature/Fix Branch
        │
        ▼
Local development + testing
        │
        ▼
Push to GitHub
        │
        ▼
CI/CD Pipeline:
  - Format check (prettier, rustfmt)
  - Lint (clippy, eslint)
  - Unit tests
  - Integration tests
  - Security audit
  - Build (all platforms)
        │
        ▼
Code review
        │
        ▼
Merge to main
        │
        ▼
Release build + signing
        │
        ▼
Distribute via CDN
```

## Future Considerations

1. **Mobile**: Flutter app for iOS/Android
2. **Linux Distro Packages**: apt, snap, AppImage
3. **macOS Signing**: Code signing + notarization
4. **Performance Mode**: Reduced animations, lower graphics
5. **Advanced Features**: Save backups, profile management
6. **Community**: Mod browser, creator tools
