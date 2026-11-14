//! Version JSON model (the format shared by Mojang, Fabric and Quilt) plus
//! rule evaluation, inheritance merging and argument building.

use crate::is_safe_relative_path;
use serde::Deserialize;
use std::collections::{HashMap, HashSet};

pub const DEFAULT_LIBRARIES_URL: &str = "https://libraries.minecraft.net/";

/// Platform and feature flags that version rules are evaluated against.
#[derive(Debug, Clone)]
pub struct Env {
    /// `windows`, `osx` or `linux`
    pub os: String,
    /// `x86`, `x86_64` or `arm64`
    pub arch: String,
    pub features: HashMap<String, bool>,
}

impl Env {
    pub fn current() -> Self {
        let os = match std::env::consts::OS {
            "windows" => "windows",
            "macos" => "osx",
            _ => "linux",
        };
        let arch = match std::env::consts::ARCH {
            "x86" => "x86",
            "aarch64" => "arm64",
            _ => "x86_64",
        };
        Self::new(os, arch)
    }

    pub fn new(os: &str, arch: &str) -> Self {
        Self {
            os: os.to_string(),
            arch: arch.to_string(),
            features: HashMap::new(),
        }
    }

    pub fn with_feature(mut self, name: &str, enabled: bool) -> Self {
        self.features.insert(name.to_string(), enabled);
        self
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VersionInfo {
    pub id: String,
    #[serde(default)]
    pub inherits_from: Option<String>,
    #[serde(default)]
    pub main_class: Option<String>,
    #[serde(default)]
    pub arguments: Option<Arguments>,
    /// Pre-1.13 argument string.
    #[serde(default)]
    pub minecraft_arguments: Option<String>,
    #[serde(default)]
    pub asset_index: Option<AssetIndexRef>,
    #[serde(default)]
    pub downloads: Option<VersionDownloads>,
    #[serde(default)]
    pub java_version: Option<JavaVersion>,
    #[serde(default)]
    pub libraries: Vec<Library>,
    #[serde(default)]
    pub logging: Option<Logging>,
    #[serde(default, rename = "type")]
    pub version_type: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct Arguments {
    #[serde(default)]
    pub game: Vec<Argument>,
    #[serde(default)]
    pub jvm: Vec<Argument>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum Argument {
    Plain(String),
    Conditional { rules: Vec<Rule>, value: ArgumentValue },
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum ArgumentValue {
    One(String),
    Many(Vec<String>),
}

#[derive(Debug, Clone, Deserialize)]
pub struct Rule {
    pub action: String,
    #[serde(default)]
    pub os: Option<OsRule>,
    #[serde(default)]
    pub features: Option<HashMap<String, bool>>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct OsRule {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub arch: Option<String>,
}

impl Rule {
    fn matches(&self, env: &Env) -> bool {
        if let Some(os) = &self.os {
            if os.name.as_ref().is_some_and(|name| *name != env.os) {
                return false;
            }
            if os.arch.as_ref().is_some_and(|arch| *arch != env.arch) {
                return false;
            }
        }
        if let Some(features) = &self.features {
            for (name, expected) in features {
                if env.features.get(name).copied().unwrap_or(false) != *expected {
                    return false;
                }
            }
        }
        true
    }
}

/// Mojang rule semantics: no rules means allowed; otherwise the last matching
/// rule decides, and nothing matching means disallowed.
pub fn rules_allow(rules: &[Rule], env: &Env) -> bool {
    if rules.is_empty() {
        return true;
    }
    let mut allowed = false;
    for rule in rules {
        if rule.matches(env) {
            allowed = rule.action == "allow";
        }
    }
    allowed
}

#[derive(Debug, Clone, Deserialize)]
pub struct AssetIndexRef {
    pub id: String,
    #[serde(default)]
    pub sha1: Option<String>,
    #[serde(default)]
    pub size: Option<u64>,
    pub url: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct VersionDownloads {
    #[serde(default)]
    pub client: Option<Artifact>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Artifact {
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub sha1: Option<String>,
    #[serde(default)]
    pub size: Option<u64>,
    pub url: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JavaVersion {
    pub component: String,
    pub major_version: u32,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Logging {
    #[serde(default)]
    pub client: Option<LoggingClient>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct LoggingClient {
    /// JVM argument containing a `${path}` placeholder.
    pub argument: String,
    pub file: LoggingFile,
}

#[derive(Debug, Clone, Deserialize)]
pub struct LoggingFile {
    pub id: String,
    #[serde(default)]
    pub sha1: Option<String>,
    #[serde(default)]
    pub size: Option<u64>,
    pub url: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Library {
    /// Maven coordinate: `group:artifact:version[:classifier][@ext]`
    pub name: String,
    #[serde(default)]
    pub downloads: Option<LibraryDownloads>,
    /// Maven repository base URL (Fabric / Quilt style libraries).
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub sha1: Option<String>,
    #[serde(default)]
    pub size: Option<u64>,
    #[serde(default)]
    pub rules: Vec<Rule>,
    /// Legacy natives: OS name -> classifier (may contain `${arch}`).
    #[serde(default)]
    pub natives: Option<HashMap<String, String>>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct LibraryDownloads {
    #[serde(default)]
    pub artifact: Option<Artifact>,
    #[serde(default)]
    pub classifiers: Option<HashMap<String, Artifact>>,
}

impl Library {
    /// `group:artifact[:classifier]`, used to let a child version override a
    /// parent's library regardless of version number.
    fn key(&self) -> String {
        let coords = self.name.split('@').next().unwrap_or(&self.name);
        let parts: Vec<&str> = coords.split(':').collect();
        match parts.as_slice() {
            [group, artifact, _, classifier, ..] => format!("{group}:{artifact}:{classifier}"),
            [group, artifact, ..] => format!("{group}:{artifact}"),
            _ => self.name.clone(),
        }
    }
}

/// Convert a Maven coordinate to its repository-relative path.
pub fn maven_path(name: &str) -> Option<String> {
    let (coords, ext) = name.split_once('@').unwrap_or((name, "jar"));
    let parts: Vec<&str> = coords.split(':').collect();
    let (group, artifact, version) = match parts.as_slice() {
        [group, artifact, version, ..] => (group, artifact, version),
        _ => return None,
    };
    let classifier = parts.get(3).map(|c| format!("-{c}")).unwrap_or_default();
    Some(format!(
        "{}/{artifact}/{version}/{artifact}-{version}{classifier}.{ext}",
        group.replace('.', "/")
    ))
}

/// Merge a version that `inheritsFrom` another (e.g. a Fabric profile) onto
/// its parent. Child values win; child libraries come first on the classpath.
pub fn merge(child: VersionInfo, parent: VersionInfo) -> VersionInfo {
    let overridden: HashSet<String> = child.libraries.iter().map(Library::key).collect();
    let mut libraries = child.libraries;
    libraries.extend(
        parent
            .libraries
            .into_iter()
            .filter(|lib| !overridden.contains(&lib.key())),
    );

    let arguments = match (parent.arguments, child.arguments) {
        (Some(mut base), Some(extra)) => {
            base.game.extend(extra.game);
            base.jvm.extend(extra.jvm);
            Some(base)
        }
        (base, extra) => base.or(extra),
    };

    VersionInfo {
        id: child.id,
        inherits_from: None,
        main_class: child.main_class.or(parent.main_class),
        arguments,
        minecraft_arguments: child.minecraft_arguments.or(parent.minecraft_arguments),
        asset_index: child.asset_index.or(parent.asset_index),
        downloads: child.downloads.or(parent.downloads),
        java_version: child.java_version.or(parent.java_version),
        libraries,
        logging: child.logging.or(parent.logging),
        version_type: child.version_type.or(parent.version_type),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LibraryKind {
    /// Goes on the classpath.
    Classpath,
    /// Legacy natives jar that must be extracted before launch.
    Native,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedLibrary {
    /// Path relative to the libraries directory.
    pub path: String,
    pub url: String,
    pub sha1: Option<String>,
    pub size: Option<u64>,
    pub kind: LibraryKind,
}

/// Resolve the libraries that apply to `env`, deduplicated by path.
/// Entries with unsafe or unresolvable paths are dropped.
pub fn resolve_libraries(info: &VersionInfo, env: &Env) -> Vec<ResolvedLibrary> {
    let mut seen = HashSet::new();
    let mut resolved = Vec::new();
    let mut push = |library: ResolvedLibrary| {
        if is_safe_relative_path(&library.path) && seen.insert(library.path.clone()) {
            resolved.push(library);
        }
    };

    for library in &info.libraries {
        if !rules_allow(&library.rules, env) {
            continue;
        }

        match &library.downloads {
            Some(downloads) => {
                if let Some(artifact) = &downloads.artifact {
                    if let Some(path) = artifact.path.clone().or_else(|| maven_path(&library.name)) {
                        push(ResolvedLibrary {
                            path,
                            url: artifact.url.clone(),
                            sha1: artifact.sha1.clone(),
                            size: artifact.size,
                            kind: LibraryKind::Classpath,
                        });
                    }
                }

                let classifier = library.natives.as_ref().and_then(|natives| natives.get(&env.os));
                if let (Some(classifier), Some(classifiers)) = (classifier, &downloads.classifiers) {
                    let bits = if env.arch == "x86" { "32" } else { "64" };
                    let classifier = classifier.replace("${arch}", bits);
                    if let Some(artifact) = classifiers.get(&classifier) {
                        let path = artifact
                            .path
                            .clone()
                            .or_else(|| maven_path(&format!("{}:{classifier}", library.name)));
                        if let Some(path) = path {
                            push(ResolvedLibrary {
                                path,
                                url: artifact.url.clone(),
                                sha1: artifact.sha1.clone(),
                                size: artifact.size,
                                kind: LibraryKind::Native,
                            });
                        }
                    }
                }
            }
            None => {
                let Some(path) = maven_path(&library.name) else {
                    continue;
                };
                let base = library.url.as_deref().unwrap_or(DEFAULT_LIBRARIES_URL);
                let separator = if base.ends_with('/') { "" } else { "/" };
                push(ResolvedLibrary {
                    url: format!("{base}{separator}{path}"),
                    path,
                    sha1: library.sha1.clone(),
                    size: library.size,
                    kind: LibraryKind::Classpath,
                });
            }
        }
    }
    resolved
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaunchArguments {
    pub jvm: Vec<String>,
    pub game: Vec<String>,
}

/// Build the JVM and game argument lists, applying rules and substituting
/// `${placeholder}` variables.
pub fn build_arguments(info: &VersionInfo, env: &Env, vars: &HashMap<String, String>) -> LaunchArguments {
    let collect = |arguments: &[Argument]| -> Vec<String> {
        let mut out = Vec::new();
        for argument in arguments {
            match argument {
                Argument::Plain(value) => out.push(substitute(value, vars)),
                Argument::Conditional { rules, value } if rules_allow(rules, env) => match value {
                    ArgumentValue::One(value) => out.push(substitute(value, vars)),
                    ArgumentValue::Many(values) => out.extend(values.iter().map(|v| substitute(v, vars))),
                },
                Argument::Conditional { .. } => {}
            }
        }
        out
    };

    let mut jvm = info.arguments.as_ref().map(|a| collect(&a.jvm)).unwrap_or_default();
    // Legacy versions (and loader profiles on top of them) carry no JVM
    // arguments of their own.
    if !jvm.iter().any(|arg| arg == "-cp" || arg == "-classpath") {
        let defaults = ["-Djava.library.path=${natives_directory}", "-cp", "${classpath}"];
        jvm.splice(0..0, defaults.iter().map(|arg| substitute(arg, vars)));
    }

    let mut game = match &info.minecraft_arguments {
        Some(legacy) => legacy.split_whitespace().map(|arg| substitute(arg, vars)).collect(),
        None => Vec::new(),
    };
    game.extend(info.arguments.as_ref().map(|a| collect(&a.game)).unwrap_or_default());

    LaunchArguments { jvm, game }
}

fn substitute(template: &str, vars: &HashMap<String, String>) -> String {
    if !template.contains("${") {
        return template.to_string();
    }
    let mut out = template.to_string();
    for (key, value) in vars {
        out = out.replace(&format!("${{{key}}}"), value);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(json: &str) -> VersionInfo {
        serde_json::from_str(json).unwrap()
    }

    const MODERN: &str = r#"{
        "id": "1.21.1",
        "mainClass": "net.minecraft.client.main.Main",
        "type": "release",
        "javaVersion": {"component": "java-runtime-delta", "majorVersion": 21},
        "assetIndex": {"id": "17", "sha1": "aa", "size": 1, "totalSize": 2, "url": "https://example/17.json"},
        "downloads": {"client": {"sha1": "bb", "size": 3, "url": "https://example/client.jar"}},
        "arguments": {
            "game": [
                "--username", "${auth_player_name}",
                {"rules": [{"action": "allow", "features": {"is_demo_user": true}}], "value": "--demo"},
                {"rules": [{"action": "allow", "features": {"has_custom_resolution": true}}],
                 "value": ["--width", "${resolution_width}"]}
            ],
            "jvm": [
                {"rules": [{"action": "allow", "os": {"name": "osx"}}], "value": ["-XstartOnFirstThread"]},
                {"rules": [{"action": "allow", "os": {"arch": "x86"}}], "value": "-Xss1M"},
                "-Djava.library.path=${natives_directory}",
                "-cp", "${classpath}"
            ]
        },
        "libraries": [
            {"name": "com.mojang:brigadier:1.3.10",
             "downloads": {"artifact": {"path": "com/mojang/brigadier/1.3.10/brigadier-1.3.10.jar",
                                        "sha1": "cc", "size": 4, "url": "https://libraries.minecraft.net/brig.jar"}}},
            {"name": "org.lwjgl:lwjgl:3.3.3:natives-windows",
             "rules": [{"action": "allow", "os": {"name": "windows"}}],
             "downloads": {"artifact": {"path": "org/lwjgl/lwjgl/3.3.3/lwjgl-3.3.3-natives-windows.jar",
                                        "sha1": "dd", "size": 5, "url": "https://libraries.minecraft.net/lwjgl-win.jar"}}},
            {"name": "org.lwjgl:lwjgl:3.3.3:natives-linux",
             "rules": [{"action": "allow", "os": {"name": "linux"}}],
             "downloads": {"artifact": {"path": "org/lwjgl/lwjgl/3.3.3/lwjgl-3.3.3-natives-linux.jar",
                                        "sha1": "ee", "size": 6, "url": "https://libraries.minecraft.net/lwjgl-linux.jar"}}},
            {"name": "evil:lib:1.0",
             "downloads": {"artifact": {"path": "../../evil.jar", "url": "https://example/evil.jar"}}}
        ]
    }"#;

    const LEGACY: &str = r#"{
        "id": "1.8.9",
        "mainClass": "net.minecraft.client.main.Main",
        "minecraftArguments": "--username ${auth_player_name} --version ${version_name}",
        "libraries": [
            {"name": "org.lwjgl.lwjgl:lwjgl-platform:2.9.4-nightly-20150209",
             "natives": {"linux": "natives-linux", "windows": "natives-windows-${arch}"},
             "rules": [{"action": "allow"}, {"action": "disallow", "os": {"name": "osx"}}],
             "downloads": {"classifiers": {
                 "natives-windows-64": {"path": "org/lwjgl/lwjgl/lwjgl-platform/2.9.4/lwjgl-platform-natives-windows-64.jar",
                                        "sha1": "ff", "size": 7, "url": "https://libraries.minecraft.net/natives-win.jar"},
                 "natives-linux": {"path": "org/lwjgl/lwjgl/lwjgl-platform/2.9.4/lwjgl-platform-natives-linux.jar",
                                   "sha1": "gg", "size": 8, "url": "https://libraries.minecraft.net/natives-linux.jar"}}}}
        ]
    }"#;

    const FABRIC: &str = r#"{
        "id": "fabric-loader-0.16.9-1.21.1",
        "inheritsFrom": "1.21.1",
        "mainClass": "net.fabricmc.loader.impl.launch.knot.KnotClient",
        "arguments": {"game": [], "jvm": ["-DFabricMcEmu= net.minecraft.client.main.Main "]},
        "libraries": [
            {"name": "net.fabricmc:fabric-loader:0.16.9", "url": "https://maven.fabricmc.net/"},
            {"name": "com.mojang:brigadier:9.9.9", "url": "https://maven.fabricmc.net"}
        ]
    }"#;

    #[test]
    fn maven_paths() {
        assert_eq!(
            maven_path("net.fabricmc:fabric-loader:0.16.9").unwrap(),
            "net/fabricmc/fabric-loader/0.16.9/fabric-loader-0.16.9.jar"
        );
        assert_eq!(
            maven_path("org.lwjgl:lwjgl:3.3.3:natives-windows").unwrap(),
            "org/lwjgl/lwjgl/3.3.3/lwjgl-3.3.3-natives-windows.jar"
        );
        assert_eq!(maven_path("a.b:c:1.0@zip").unwrap(), "a/b/c/1.0/c-1.0.zip");
        assert!(maven_path("broken").is_none());
    }

    #[test]
    fn rule_semantics() {
        let rules: Vec<Rule> =
            serde_json::from_str(r#"[{"action": "allow"}, {"action": "disallow", "os": {"name": "osx"}}]"#).unwrap();
        assert!(rules_allow(&rules, &Env::new("windows", "x86_64")));
        assert!(!rules_allow(&rules, &Env::new("osx", "x86_64")));

        let only_osx: Vec<Rule> = serde_json::from_str(r#"[{"action": "allow", "os": {"name": "osx"}}]"#).unwrap();
        assert!(!rules_allow(&only_osx, &Env::new("linux", "x86_64")));
        assert!(rules_allow(&[], &Env::new("linux", "x86_64")));
    }

    #[test]
    fn modern_libraries_filtered_by_os_and_safety() {
        let info = parse(MODERN);
        let libs = resolve_libraries(&info, &Env::new("windows", "x86_64"));
        let paths: Vec<&str> = libs.iter().map(|l| l.path.as_str()).collect();
        assert_eq!(
            paths,
            vec![
                "com/mojang/brigadier/1.3.10/brigadier-1.3.10.jar",
                "org/lwjgl/lwjgl/3.3.3/lwjgl-3.3.3-natives-windows.jar"
            ]
        );
        assert!(libs.iter().all(|l| l.kind == LibraryKind::Classpath));
    }

    #[test]
    fn legacy_natives_resolved_per_platform() {
        let info = parse(LEGACY);

        let windows = resolve_libraries(&info, &Env::new("windows", "x86_64"));
        assert_eq!(windows.len(), 1);
        assert_eq!(windows[0].kind, LibraryKind::Native);
        assert!(windows[0].path.ends_with("natives-windows-64.jar"));

        assert!(resolve_libraries(&info, &Env::new("osx", "x86_64")).is_empty());
    }

    #[test]
    fn modern_arguments() {
        let info = parse(MODERN);
        let vars = HashMap::from([
            ("auth_player_name".to_string(), "Steve".to_string()),
            ("natives_directory".to_string(), "/n".to_string()),
            ("classpath".to_string(), "a.jar".to_string()),
            ("resolution_width".to_string(), "1280".to_string()),
        ]);

        let args = build_arguments(&info, &Env::new("windows", "x86_64"), &vars);
        assert_eq!(args.jvm, vec!["-Djava.library.path=/n", "-cp", "a.jar"]);
        assert_eq!(args.game, vec!["--username", "Steve"]);

        let env = Env::new("osx", "x86_64").with_feature("has_custom_resolution", true);
        let args = build_arguments(&info, &env, &vars);
        assert_eq!(args.jvm[0], "-XstartOnFirstThread");
        assert_eq!(args.game, vec!["--username", "Steve", "--width", "1280"]);
    }

    #[test]
    fn legacy_arguments_get_default_jvm_args() {
        let info = parse(LEGACY);
        let vars = HashMap::from([
            ("auth_player_name".to_string(), "Alex".to_string()),
            ("version_name".to_string(), "1.8.9".to_string()),
            ("natives_directory".to_string(), "/n".to_string()),
            ("classpath".to_string(), "a.jar".to_string()),
        ]);
        let args = build_arguments(&info, &Env::new("linux", "x86_64"), &vars);
        assert_eq!(args.jvm, vec!["-Djava.library.path=/n", "-cp", "a.jar"]);
        assert_eq!(args.game, vec!["--username", "Alex", "--version", "1.8.9"]);
    }

    #[test]
    fn fabric_profile_merges_onto_vanilla() {
        let merged = merge(parse(FABRIC), parse(MODERN));
        assert_eq!(merged.id, "fabric-loader-0.16.9-1.21.1");
        assert_eq!(
            merged.main_class.as_deref(),
            Some("net.fabricmc.loader.impl.launch.knot.KnotClient")
        );
        assert_eq!(merged.java_version.as_ref().unwrap().major_version, 21);
        assert!(merged.downloads.is_some());

        let libs = resolve_libraries(&merged, &Env::new("windows", "x86_64"));
        // Loader first; the child's brigadier replaces the vanilla one.
        assert_eq!(
            libs[0].url,
            "https://maven.fabricmc.net/net/fabricmc/fabric-loader/0.16.9/fabric-loader-0.16.9.jar"
        );
        assert_eq!(
            libs[1].url,
            "https://maven.fabricmc.net/com/mojang/brigadier/9.9.9/brigadier-9.9.9.jar"
        );
        assert_eq!(libs.iter().filter(|l| l.path.contains("brigadier")).count(), 1);
        // Both native variants of the parent survive the merge.
        assert_eq!(libs.len(), 3);

        let args = build_arguments(&merged, &Env::new("windows", "x86_64"), &HashMap::new());
        assert!(args.jvm.contains(&"-cp".to_string()));
        assert_eq!(
            args.jvm.last().unwrap(),
            "-DFabricMcEmu= net.minecraft.client.main.Main "
        );
    }
}
