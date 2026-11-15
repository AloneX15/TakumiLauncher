//! TakumiLauncher - Minecraft Version Resolver
//!
//! Parses Mojang / Fabric / Quilt version metadata and resolves it into the
//! concrete libraries and arguments needed to launch the game.

use thiserror::Error;

pub mod meta;
pub mod version;

pub use meta::*;
pub use version::*;

#[derive(Error, Debug)]
pub enum ResolverError {
    #[error("Versión no encontrada: {0}")]
    VersionNotFound(String),

    #[error("Error de red: {0}")]
    NetworkError(String),

    #[error("Metadatos no válidos: {0}")]
    ParseError(String),
}

pub type ResolverResult<T> = Result<T, ResolverError>;

/// Whether `path` is a plain relative path that cannot escape its base
/// directory. Applied to every path taken from remote metadata.
///
/// Besides `..`, absolute paths and drive letters, it rejects what Windows
/// treats specially: device names (`CON`, `NUL`, `COM1`…, with or without an
/// extension), components ending in a dot or space (Win32 strips them, so
/// `".. "` would mean `".."`), control characters and `<>"|?*`.
pub fn is_safe_relative_path(path: &str) -> bool {
    !path.is_empty()
        && !path.starts_with(['/', '\\'])
        && !path
            .chars()
            .any(|c| c.is_control() || matches!(c, ':' | '<' | '>' | '"' | '|' | '?' | '*'))
        && path.split(['/', '\\']).all(is_safe_component)
}

fn is_safe_component(part: &str) -> bool {
    !part.is_empty() && part != "." && part != ".." && !part.ends_with(['.', ' ']) && !is_windows_device_name(part)
}

/// `CON`, `PRN`, `AUX`, `NUL`, `COM0-9`, `LPT0-9` (also `COM¹`-style
/// superscripts), case-insensitive, with or without an extension.
fn is_windows_device_name(part: &str) -> bool {
    let stem = part.split('.').next().unwrap_or(part).trim_end().to_ascii_uppercase();
    match stem.as_str() {
        "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$" => true,
        _ => {
            let mut chars = stem.chars();
            let prefix: String = chars.by_ref().take(3).collect();
            let rest: String = chars.collect();
            (prefix == "COM" || prefix == "LPT")
                && rest.chars().count() == 1
                && rest.chars().all(|c| c.is_ascii_digit() || matches!(c, '¹' | '²' | '³'))
        }
    }
}

/// Whether `name` is usable as a single directory / file name.
pub fn is_safe_file_name(name: &str) -> bool {
    is_safe_relative_path(name) && !name.contains(['/', '\\'])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn safe_paths() {
        assert!(is_safe_relative_path("com/mojang/a/1.0/a-1.0.jar"));
        assert!(is_safe_relative_path("bin/java.exe"));
        assert!(!is_safe_relative_path("../evil"));
        assert!(!is_safe_relative_path("a/../../evil"));
        assert!(!is_safe_relative_path("/etc/passwd"));
        assert!(!is_safe_relative_path("C:\\Windows\\x"));
        assert!(!is_safe_relative_path(""));
        assert!(!is_safe_relative_path("a//b"));

        assert!(is_safe_file_name("1.14 Pre-Release 5"));
        assert!(!is_safe_file_name("a/b"));
        assert!(!is_safe_file_name(".."));
    }

    #[test]
    fn windows_special_names() {
        for bad in [
            ".. /evil",
            "a/.. /b",
            "mods/x.jar.",
            "mods/x.jar ",
            "CON",
            "nul.txt",
            "mods/aux.jar",
            "com1",
            "LPT9.log",
            "COM¹",
            "a\0b",
            "a\tb",
            "file:stream",
            "a?b",
            "a*b",
            "a|b",
            "a<b",
            "a>b",
            "a\"b",
        ] {
            assert!(!is_safe_relative_path(bad), "{bad:?}");
        }
        for good in [
            "config/console.json",
            "mods/nullpointer.jar",
            "com/mojang/patchy/1.3.9/patchy-1.3.9.jar",
            "COM10",
            "conin",
            "minecraft/sounds/ambient/cave/cave1.ogg",
            "resourcepacks/Faithful 32x.zip",
        ] {
            assert!(is_safe_relative_path(good), "{good:?}");
        }
    }
}
