# Contributing to TakumiLauncher

Thank you for interest in contributing! This document explains how to contribute effectively.

## Code of Conduct

Be respectful, inclusive, and professional. We maintain a positive community.

## How to Contribute

### Report Bugs

1. Check if bug is already reported
2. Include:
   - OS and version
   - TakumiLauncher version
   - Steps to reproduce
   - Expected vs actual behavior
   - Screenshots if applicable
   - Logs from `~/.takumilauncher/logs/`

### Suggest Features

1. Check if suggestion already exists
2. Explain use case
3. Suggest implementation (if you have ideas)
4. Gather community feedback

### Submit Code

1. **Fork** the repository
2. **Create branch**: `git checkout -b feature/your-feature`
3. **Code**: Follow style guide (see below)
4. **Test**: Add tests for your changes
5. **Commit**: Clear commit messages
6. **Push**: To your fork
7. **Pull Request**: Describe changes and reference issues

## Development Setup

See [docs/DEVELOPMENT.md](docs/DEVELOPMENT.md) for detailed setup instructions.

Quick start:
```bash
cd takumi-launcher
just setup
just test
```

## Code Style

### Rust

```bash
# Format
cargo fmt --all

# Check
cargo clippy --workspace

# Follow Rust API Guidelines
https://rust-lang.github.io/api-guidelines/
```

### TypeScript/JavaScript

```bash
# Format
pnpm format

# Lint
pnpm lint

# Follow Airbnb style guide
```

### General

- **Comments**: Explain *why*, not *what*
- **Names**: Clear, descriptive, no abbreviations (except common ones)
- **Functions**: Small, single responsibility
- **Documentation**: Keep README and docs updated

## Testing

### Write Tests

- Unit tests for all functions
- Integration tests for workflows
- Test edge cases and error conditions

```bash
# Run all tests
just test

# Run specific test
cargo test -p minecraft-resolver

# With output
cargo test -- --nocapture
```

### Test Checklist

- [ ] Unit tests pass
- [ ] Integration tests pass
- [ ] No warnings from clippy/eslint
- [ ] Code coverage > 70%
- [ ] Manual testing on Windows/Linux/macOS

## Commit Guidelines

```
<type>: <subject>

<body>

<footer>
```

### Type
- `feat`: New feature
- `fix`: Bug fix
- `docs`: Documentation
- `style`: Code style (formatting)
- `refactor`: Code refactoring
- `test`: Tests
- `chore`: Build, deps, etc

### Example
```
feat: add Modrinth modpack search

Allow users to search and filter modpacks from Modrinth directly in the launcher.
Implements full dependency resolution for imported modpacks.

Fixes #123
Closes #456
```

## Pull Request Process

1. **Title**: Clear, descriptive
2. **Description**: What, why, how
3. **Testing**: How to test changes
4. **Screenshots**: For UI changes
5. **Checklist**:
   - [ ] Tests pass
   - [ ] Code formatted
   - [ ] Documentation updated
   - [ ] No breaking changes (or documented)
6. **Link issues**: Reference related issues

## Review Process

- Core team reviews for:
  - Code quality
  - Security implications
  - Performance impact
  - Test coverage
  - Documentation

- Feedback aims to improve, not criticize
- Iterations are normal
- Once approved, PR is merged

## Architecture Decisions

Major changes should discuss architecture first:

1. Open an issue for discussion
2. Propose design/architecture
3. Get feedback from core team
4. Implement once consensus reached

This prevents wasted effort and ensures consistency.

## Documentation

### Update These When Needed

- `README.md` - Project overview
- `docs/ARCHITECTURE.md` - System design
- `docs/DEVELOPMENT.md` - Setup and development
- `docs/API.md` - API endpoints (when added)
- Code comments - Inline documentation

### Documentation Standard

- Clear, professional language
- Markdown format
- Examples where applicable
- Keep updated with code changes

## Reporting Security Issues

**DO NOT** post security vulnerabilities publicly.

See [docs/SECURITY.md](docs/SECURITY.md) for responsible disclosure process.

## License

By contributing, you agree your code is licensed under MIT.

## Questions?

- Check [docs/](docs/) first
- Open GitHub discussion
- Email: dev@takumilabs.com

---

Thank you for contributing to TakumiLauncher! 🚀
