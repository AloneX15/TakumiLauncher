//! TakumiLauncher - Manifest System
//!
//! Signed manifests of private instances (TakumiWeb `/instancias`). A manifest lists every file the game
//! folder of one published version must contain. The website signs the exact JSON text with Ed25519; the
//! launcher verifies the signature against the public key it was built with before trusting any field,
//! then validates every path so a manifest can never write outside the instance folder.
//!
//! Keep in sync with `src/lib/server/instances/manifest.ts` in takumi-webpage.

pub mod vault;

use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine;
use minecraft_resolver::is_safe_relative_path;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum ManifestError {
    #[error("Manifiesto no válido: {0}")]
    Invalid(String),

    #[error("La firma del manifiesto no es válida: no viene del servidor de Takumi o ha sido modificado.")]
    SignatureVerificationFailed,

    #[error("Clave pública de manifiestos no válida.")]
    BadPublicKey,
}

pub type ManifestResult<T> = Result<T, ManifestError>;

/// Only format understood by this launcher.
pub const FORMAT: u32 = 1;

/// Folders an organizer can lock (mirrors `LOCKABLE_DIRS` on the website).
pub const LOCKABLE_DIRS: [&str; 5] = ["mods", "resourcepacks", "shaderpacks", "config", "datapacks"];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FilePolicy {
    /// Always identical to the manifest: replaced when it differs.
    Enforced,
    /// Copied only when missing, so players keep their own changes (options.txt).
    Default,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManifestFile {
    pub path: String,
    pub sha256: String,
    pub size: u64,
    pub policy: FilePolicy,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PrivateManifest {
    pub format: u32,
    pub instance: String,
    pub name: String,
    pub version: u32,
    pub minecraft: String,
    pub loader: String,
    pub loader_version: Option<String>,
    pub server: Option<String>,
    pub locked_dirs: Vec<String>,
    pub restart_required: bool,
    pub grace_minutes: u32,
    pub published_at: String,
    pub files: Vec<ManifestFile>,
}

impl PrivateManifest {
    /// Whether `path` (relative, `/`-separated) lies in a locked folder.
    pub fn is_locked(&self, path: &str) -> bool {
        let top = path.split('/').next().unwrap_or("");
        path.contains('/') && self.locked_dirs.iter().any(|dir| dir == top)
    }
}

/// Verify `signature_b64` (Ed25519 over the UTF-8 bytes of `text`) with the raw 32-byte public key in
/// `public_key_b64`, then parse and validate the manifest.
pub fn verify_and_parse(text: &str, signature_b64: &str, public_key_b64: &str) -> ManifestResult<PrivateManifest> {
    let key = BASE64
        .decode(public_key_b64.trim())
        .map_err(|_| ManifestError::BadPublicKey)?;
    if key.len() != 32 {
        return Err(ManifestError::BadPublicKey);
    }
    let signature = BASE64
        .decode(signature_b64.trim())
        .map_err(|_| ManifestError::SignatureVerificationFailed)?;
    ring::signature::UnparsedPublicKey::new(&ring::signature::ED25519, &key)
        .verify(text.as_bytes(), &signature)
        .map_err(|_| ManifestError::SignatureVerificationFailed)?;
    let manifest: PrivateManifest =
        serde_json::from_str(text).map_err(|error| ManifestError::Invalid(error.to_string()))?;
    validate(&manifest)?;
    Ok(manifest)
}

/// Checks a parsed manifest: known format, safe unique paths, well-formed hashes, known locked folders.
pub fn validate(manifest: &PrivateManifest) -> ManifestResult<()> {
    let invalid = |message: String| Err(ManifestError::Invalid(message));
    if manifest.format != FORMAT {
        return invalid(format!(
            "formato {} no soportado; actualiza Takumi Launcher",
            manifest.format
        ));
    }
    for dir in &manifest.locked_dirs {
        if !LOCKABLE_DIRS.contains(&dir.as_str()) {
            return invalid(format!("carpeta bloqueada desconocida: {dir}"));
        }
    }
    let mut seen = HashSet::new();
    for file in &manifest.files {
        if !is_safe_path(&file.path) {
            return invalid(format!("ruta no permitida: {}", file.path));
        }
        if file.sha256.len() != 64 || !file.sha256.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')) {
            return invalid(format!("hash no válido para {}", file.path));
        }
        if !seen.insert(file.path.to_ascii_lowercase()) {
            return invalid(format!("ruta repetida: {}", file.path));
        }
    }
    Ok(())
}

/// A relative `/`-separated path that stays inside the game folder on every OS. Paths that only differ in
/// case are treated as the same file (Windows and macOS folders are case-insensitive).
pub fn is_safe_path(path: &str) -> bool {
    path.len() <= 512 && !path.contains('\\') && is_safe_relative_path(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ring::rand::SystemRandom;
    use ring::signature::{Ed25519KeyPair, KeyPair};

    fn keypair() -> (Ed25519KeyPair, String) {
        let pkcs8 = Ed25519KeyPair::generate_pkcs8(&SystemRandom::new()).unwrap();
        let pair = Ed25519KeyPair::from_pkcs8(pkcs8.as_ref()).unwrap();
        let public = BASE64.encode(pair.public_key().as_ref());
        (pair, public)
    }

    fn manifest_json(files: &str, locked: &str) -> String {
        format!(
            r#"{{"format":1,"instance":"i","name":"Evento","version":3,"minecraft":"1.21.1","loader":"fabric","loader_version":null,"server":"play.example.com","locked_dirs":[{locked}],"restart_required":true,"grace_minutes":5,"published_at":"2026-11-15T00:00:00.000Z","files":[{files}]}}"#
        )
    }

    const FILE: &str = r#"{"path":"mods/a.jar","sha256":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","size":3,"policy":"enforced"}"#;

    fn signed(text: &str, pair: &Ed25519KeyPair) -> String {
        BASE64.encode(pair.sign(text.as_bytes()).as_ref())
    }

    #[test]
    fn accepts_a_valid_signed_manifest() {
        let (pair, public) = keypair();
        let text = manifest_json(FILE, r#""mods""#);
        let manifest = verify_and_parse(&text, &signed(&text, &pair), &public).unwrap();
        assert_eq!(manifest.version, 3);
        assert_eq!(manifest.files[0].policy, FilePolicy::Enforced);
        assert!(manifest.is_locked("mods/other.jar"));
        assert!(!manifest.is_locked("config/x.json"));
        assert!(!manifest.is_locked("mods"));
    }

    #[test]
    fn rejects_tampering_and_other_keys() {
        let (pair, public) = keypair();
        let (_, other_public) = keypair();
        let text = manifest_json(FILE, r#""mods""#);
        let signature = signed(&text, &pair);
        let tampered = text.replace("Evento", "Evento!");
        assert!(matches!(
            verify_and_parse(&tampered, &signature, &public),
            Err(ManifestError::SignatureVerificationFailed)
        ));
        assert!(matches!(
            verify_and_parse(&text, &signature, &other_public),
            Err(ManifestError::SignatureVerificationFailed)
        ));
        assert!(matches!(
            verify_and_parse(&text, &signature, "AAAA"),
            Err(ManifestError::BadPublicKey)
        ));
    }

    #[test]
    fn rejects_unsafe_content_even_when_signed() {
        let (pair, public) = keypair();
        for (files, locked) in [
            (FILE.replace("mods/a.jar", "../evil.jar"), r#""mods""#.to_string()),
            (FILE.replace("mods/a.jar", "mods\\\\a.jar"), r#""mods""#.to_string()),
            (FILE.replace("mods/a.jar", "/abs.jar"), r#""mods""#.to_string()),
            (FILE.replace("aaaa", "AAAA"), r#""mods""#.to_string()),
            (
                format!("{FILE},{}", FILE.replace("mods/a.jar", "MODS/A.jar")),
                r#""mods""#.to_string(),
            ),
            (FILE.to_string(), r#""saves""#.to_string()),
        ] {
            let text = manifest_json(&files, &locked);
            let result = verify_and_parse(&text, &signed(&text, &pair), &public);
            assert!(matches!(result, Err(ManifestError::Invalid(_))), "{files} {locked}");
        }
    }

    #[test]
    fn rejects_unknown_format() {
        let (pair, public) = keypair();
        let text = manifest_json(FILE, "").replace(r#""format":1"#, r#""format":2"#);
        assert!(matches!(
            verify_and_parse(&text, &signed(&text, &pair), &public),
            Err(ManifestError::Invalid(_))
        ));
    }
}
