//! Remote metadata: version manifest, mod loader versions, asset indexes and
//! Mojang's Java runtime distribution.

use crate::version::Artifact;
use crate::{ResolverError, ResolverResult};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

pub const VERSION_MANIFEST_URL: &str = "https://piston-meta.mojang.com/mc/game/version_manifest_v2.json";
pub const JAVA_RUNTIME_URL: &str =
    "https://piston-meta.mojang.com/v1/products/java-runtime/2ec0cc96c44e5a76b9c8b7c39df7210883d12871/all.json";
pub const ASSET_BASE_URL: &str = "https://resources.download.minecraft.net/";

pub async fn fetch_text(client: &reqwest::Client, url: &str) -> ResolverResult<String> {
    let net = |e: reqwest::Error| ResolverError::NetworkError(format!("{url}: {e}"));
    client
        .get(url)
        .send()
        .await
        .map_err(net)?
        .error_for_status()
        .map_err(net)?
        .text()
        .await
        .map_err(net)
}

pub async fn fetch_json<T: DeserializeOwned>(client: &reqwest::Client, url: &str) -> ResolverResult<T> {
    let text = fetch_text(client, url).await?;
    serde_json::from_str(&text).map_err(|e| ResolverError::ParseError(format!("{url}: {e}")))
}

/// Official list of Minecraft versions
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VersionManifest {
    pub latest: LatestVersions,
    pub versions: Vec<VersionEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LatestVersions {
    pub release: String,
    pub snapshot: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VersionEntry {
    pub id: String,
    /// `release`, `snapshot`, `old_beta` or `old_alpha`
    #[serde(rename = "type")]
    pub kind: String,
    pub url: String,
    pub release_time: String,
    #[serde(default)]
    pub sha1: Option<String>,
}

impl VersionManifest {
    pub fn find(&self, id: &str) -> ResolverResult<&VersionEntry> {
        self.versions
            .iter()
            .find(|v| v.id == id)
            .ok_or_else(|| ResolverError::VersionNotFound(id.to_string()))
    }
}

/// Mod loaders installable from a launcher profile JSON.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoaderKind {
    Fabric,
    Quilt,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct LoaderVersion {
    pub version: String,
    pub stable: bool,
}

impl LoaderKind {
    pub fn slug(self) -> &'static str {
        match self {
            LoaderKind::Fabric => "fabric",
            LoaderKind::Quilt => "quilt",
        }
    }

    fn base_url(self) -> &'static str {
        match self {
            LoaderKind::Fabric => "https://meta.fabricmc.net/v2",
            LoaderKind::Quilt => "https://meta.quiltmc.org/v3",
        }
    }

    /// Version id of the installed profile, e.g. `fabric-loader-0.16.9-1.21.1`.
    pub fn profile_id(self, minecraft: &str, loader: &str) -> String {
        format!("{}-loader-{loader}-{minecraft}", self.slug())
    }

    pub fn profile_url(self, minecraft: &str, loader: &str) -> String {
        format!(
            "{}/versions/loader/{}/{}/profile/json",
            self.base_url(),
            encode_segment(minecraft),
            encode_segment(loader)
        )
    }

    /// Loader versions available for a Minecraft version, newest first.
    /// Empty when the loader does not support that version.
    pub async fn versions(self, client: &reqwest::Client, minecraft: &str) -> ResolverResult<Vec<LoaderVersion>> {
        #[derive(Deserialize)]
        struct Entry {
            loader: Loader,
        }
        #[derive(Deserialize)]
        struct Loader {
            version: String,
            stable: Option<bool>,
        }

        let url = format!("{}/versions/loader/{}", self.base_url(), encode_segment(minecraft));
        let entries: Vec<Entry> = fetch_json(client, &url).await?;
        Ok(entries
            .into_iter()
            .map(|entry| LoaderVersion {
                // Quilt has no `stable` flag; pre-releases carry a suffix.
                stable: entry.loader.stable.unwrap_or(!entry.loader.version.contains('-')),
                version: entry.loader.version,
            })
            .collect())
    }
}

/// Percent-encode a URL path segment (version ids may contain spaces).
fn encode_segment(segment: &str) -> String {
    let mut out = String::with_capacity(segment.len());
    for byte in segment.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' | b'+' => out.push(byte as char),
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

/// Asset index (`assets/indexes/<id>.json`)
#[derive(Debug, Clone, Deserialize)]
pub struct AssetIndex {
    pub objects: HashMap<String, AssetObject>,
    /// Pre-1.7.3: assets are read from a plain directory tree.
    #[serde(default, rename = "virtual")]
    pub is_virtual: bool,
    /// Pre-1.6: assets are read from `<game dir>/resources`.
    #[serde(default)]
    pub map_to_resources: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AssetObject {
    pub hash: String,
    pub size: u64,
}

impl AssetObject {
    /// `ab/abcdef...`, or `None` if the hash is not a SHA-1 hex string.
    pub fn relative_path(&self) -> Option<String> {
        let valid = self.hash.len() == 40 && self.hash.bytes().all(|b| b.is_ascii_hexdigit());
        valid.then(|| format!("{}/{}", &self.hash[..2], self.hash))
    }
}

/// `platform -> component -> releases`
pub type JavaRuntimeIndex = HashMap<String, HashMap<String, Vec<JavaRuntimeEntry>>>;

#[derive(Debug, Clone, Deserialize)]
pub struct JavaRuntimeEntry {
    pub manifest: Artifact,
}

#[derive(Debug, Clone, Deserialize)]
pub struct JavaRuntimeManifest {
    pub files: HashMap<String, JavaRuntimeFile>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct JavaRuntimeFile {
    /// `file`, `directory` or `link`
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default)]
    pub executable: bool,
    #[serde(default)]
    pub downloads: Option<JavaRuntimeDownloads>,
    /// Link target, relative to the link's directory.
    #[serde(default)]
    pub target: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct JavaRuntimeDownloads {
    #[serde(default)]
    pub raw: Option<Artifact>,
}

/// Key of the current platform in [`JavaRuntimeIndex`].
pub fn java_runtime_platform() -> Option<&'static str> {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("windows", "x86_64") => Some("windows-x64"),
        ("windows", "x86") => Some("windows-x86"),
        ("windows", "aarch64") => Some("windows-arm64"),
        ("linux", "x86_64") => Some("linux"),
        ("linux", "x86") => Some("linux-i386"),
        ("macos", "x86_64") => Some("mac-os"),
        ("macos", "aarch64") => Some("mac-os-arm64"),
        _ => None,
    }
}

/// Path of the Java executable inside an installed runtime.
pub fn java_executable_path() -> &'static str {
    match std::env::consts::OS {
        "windows" => "bin/javaw.exe",
        "macos" => "jre.bundle/Contents/Home/bin/java",
        _ => "bin/java",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_version_manifest() {
        let manifest: VersionManifest = serde_json::from_str(
            r#"{"latest": {"release": "1.21.1", "snapshot": "24w33a"},
                "versions": [{"id": "1.21.1", "type": "release", "url": "https://x/1.21.1.json",
                              "time": "t", "releaseTime": "2024-08-08T12:24:45+00:00",
                              "sha1": "abc", "complianceLevel": 1}]}"#,
        )
        .unwrap();
        assert_eq!(manifest.find("1.21.1").unwrap().kind, "release");
        assert!(manifest.find("nope").is_err());
    }

    #[test]
    fn loader_urls() {
        assert_eq!(
            LoaderKind::Fabric.profile_id("1.21.1", "0.16.9"),
            "fabric-loader-0.16.9-1.21.1"
        );
        assert_eq!(
            LoaderKind::Quilt.profile_url("1.14 Pre-Release 5", "0.27.1"),
            "https://meta.quiltmc.org/v3/versions/loader/1.14%20Pre-Release%205/0.27.1/profile/json"
        );
    }

    #[test]
    fn asset_object_path_requires_sha1() {
        let ok = AssetObject {
            hash: "bdf48ef6b5d0d23bbb02e17d04865216179f510a".into(),
            size: 1,
        };
        assert_eq!(
            ok.relative_path().unwrap(),
            "bd/bdf48ef6b5d0d23bbb02e17d04865216179f510a"
        );
        let bad = AssetObject {
            hash: "../../../etc".into(),
            size: 1,
        };
        assert!(bad.relative_path().is_none());
    }

    #[test]
    fn parses_java_runtime_manifest() {
        let manifest: JavaRuntimeManifest = serde_json::from_str(
            r#"{"files": {
                "bin": {"type": "directory"},
                "bin/java": {"type": "file", "executable": true,
                             "downloads": {"lzma": {"sha1": "a", "size": 1, "url": "https://x/l"},
                                           "raw": {"sha1": "b", "size": 2, "url": "https://x/r"}}},
                "lib/link": {"type": "link", "target": "../bin/java"}}}"#,
        )
        .unwrap();
        let java = &manifest.files["bin/java"];
        assert!(java.executable);
        assert_eq!(java.downloads.as_ref().unwrap().raw.as_ref().unwrap().size, Some(2));
        assert_eq!(manifest.files["lib/link"].target.as_deref(), Some("../bin/java"));
    }
}
