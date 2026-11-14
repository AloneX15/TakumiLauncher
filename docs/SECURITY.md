# TakumiLauncher - Security Policy

## Overview

Security is a core priority in TakumiLauncher development. This document outlines our security practices and threat model.

## Principles

1. **Defense in Depth**: Multiple layers of security
2. **Secure by Default**: Safe defaults, opt-in for risky features
3. **Least Privilege**: Minimal permissions required
4. **Transparency**: Clear communication about security practices

## Threat Model

### High-Impact Threats

| Threat | Impact | Mitigation |
|--------|--------|-----------|
| **MITM Attack** | Intercept downloads, inject malware | HTTPS + TLS 1.3, certificate pinning |
| **Compromised CDN** | Serve malicious files | SHA256 hash verification |
| **Malicious Manifest** | Update to broken state | Digital signature verification |
| **Token Theft** | Unauthorized access | Secure OS Keychain storage |
| **Path Traversal** | Write outside instance dir | Strict path validation |
| **Supply Chain Attack** | Compromised dependency | Regular audits, minimal deps |

### Mitigation Strategies

#### Network Security

```
User Request
    │
    ▼
TLS 1.3 Encryption (HTTPS)
    │
    ├─→ Certificate pinning for critical endpoints
    ├─→ Header validation (User-Agent, etc)
    │
    ▼
Rate limiting + bot detection
    │
    ▼
Authenticated endpoint (if required)
    │
    ├─→ JWT verification
    ├─→ Token expiration check
    │
    ▼
Rate limit per user
    │
    ▼
Backend processing
```

#### File Integrity

```
File Download
    │
    ▼
Verify SHA256 hash
    │
    ├─→ Mismatch: Reject + retry
    ├─→ Success: Continue
    │
    ▼
Optional: Virus scan (ClamAV)
    │
    ▼
Move to cache
```

#### Update Security

```
Check for update: GET /api/v1/instances/{id}/update
    │
    ▼
Receive manifest (signed)
    │
    ▼
Verify signature (RSA-2048)
    │
    ├─→ Invalid: Reject update
    ├─→ Valid: Continue
    │
    ▼
Verify manifest timestamp (prevent downgrades)
    │
    ▼
Download files to staging/
    │
    ▼
Verify all file hashes
    │
    ├─→ Any mismatch: Reject
    ├─→ All valid: Continue
    │
    ▼
Atomic apply (atomic file ops or backup/restore)
```

## Implementation Details

### Secure Token Storage

**Windows**
- Use Windows Credential Manager (DPAPI encryption)
- Store JWT in secure credential
- Automatic OS-level protection

**macOS**
- Use Keychain Services
- Store JWT with kSecAttrAccessibleAfterFirstUnlock
- Automatic encryption at rest

**Linux**
- Use Secret Service (D-Bus based)
- Fallback to pass/gpg if unavailable
- Never store in plaintext

### Password Hashing

- Algorithm: Argon2id
- Memory: 19 MiB
- Time: 2 iterations
- Parallelism: 1 (desktop only)
- Salt: 16 bytes (generated per user)

### JWT Implementation

```
Header:
{
  "alg": "RS256",
  "typ": "JWT",
  "kid": "key-id" // Key rotation support
}

Payload:
{
  "sub": "user-id",
  "email": "user@example.com",
  "roles": ["user"],
  "exp": 3600,
  "iat": 1234567890
}

Signature: RS256(header + payload, private_key)
```

### Manifest Signing

```
Manifest + Metadata
    │
    ▼
SHA256(manifest JSON)
    │
    ▼
Sign with RSA-2048 private key
    │
    ▼
Embed signature in manifest
    │
    ▼
Publish to CDN

---

Launcher receives manifest
    │
    ▼
Extract signature
    │
    ▼
Verify with public key (embedded)
    │
    ├─→ Failed: Reject manifest
    ├─→ Success: Continue
    │
    ▼
Verify timestamp (no downgrades)
```

### Path Validation

```
Manifest includes: path = "mods/example.jar"
    │
    ▼
Validate path:
  ✓ No "../" sequences
  ✓ No absolute paths
  ✓ No symlinks (follow check)
  ✓ No reserved Windows names (CON, PRN, etc)
  ✓ Resolve to within instance directory
    │
    ├─→ Failed: Reject file entry
    ├─→ Success: Download to instance
```

## Security Best Practices for Users

### Account Security

1. **Use strong passwords**: 16+ characters, mixed case, symbols
2. **Enable MFA**: When available (planned for phase 2)
3. **Secure your device**: Keep OS updated
4. **Beware of phishing**: Never share credentials

### Mod Safety

1. **Download from trusted sources**: Modrinth, CurseForge
2. **Check mod version compatibility**: Don't force incompatible mods
3. **Verify mod signatures**: When available
4. **Keep mods updated**: Security patches are important

### Instance Management

1. **Regular backups**: Use built-in backup feature
2. **Monitor disk space**: Prevent corruption
3. **Verify integrity**: Use health check feature
4. **Report issues**: Found a security bug? See below

## Security Issues

### Reporting

**DO NOT** open a public GitHub issue for security vulnerabilities.

Instead, email: **security@takumilabs.com**

Include:
- Description of vulnerability
- Steps to reproduce
- Potential impact
- Your contact information (optional)

We will:
1. Acknowledge receipt within 48 hours
2. Investigate and confirm
3. Develop fix
4. Coordinate disclosure timing
5. Credit in release notes (if desired)

### Responsible Disclosure Timeline

- Day 0: Report received
- Day 1-7: Initial assessment
- Day 7-30: Fix development
- Day 30: Fix released
- Day 31+: Public disclosure

## Security Checklist (for developers)

### Before Committing

- [ ] No hardcoded secrets
- [ ] No debug logs in production code
- [ ] No insecure deserialization
- [ ] Input validation on all endpoints
- [ ] Output encoding for display
- [ ] HTTPS used everywhere
- [ ] Error messages don't leak info

### Before Release

- [ ] Security audit completed
- [ ] Dependency audit passed (`cargo audit`)
- [ ] Code review by security-conscious reviewer
- [ ] Penetration testing plan documented
- [ ] Privacy policy reviewed
- [ ] Terms of service reviewed

## Dependencies

### Rust Security

We use minimal dependencies with preference for:
- Well-maintained crates
- Regular security updates
- Minimal transitive dependencies

Current audit: `cargo audit`

### Node/TypeScript Security

Similar principles apply:
- `pnpm audit` before release
- Regular updates
- Minimal dev dependencies in production

## Compliance

### Privacy

- GDPR compliant
- CCPA considerations
- No unnecessary data collection
- Clear privacy policy

### Licensing

- All dependencies properly licensed
- Respect mod author rights
- Proper attribution
- No license violations

## Contact

- **Security Issues**: security@takumilabs.com
- **Privacy Concerns**: privacy@takumilabs.com
- **General Questions**: support@takumilabs.com

---

**Last Updated**: 2026-09-16
**Version**: 0.1.0
