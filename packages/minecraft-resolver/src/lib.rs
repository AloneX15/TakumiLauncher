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
pub fn is_safe_relative_path(path: &str) -> bool {
    !path.is_empty()
        && !path.contains(':')
        && !path.starts_with(['/', '\\'])
        && path
            .split(['/', '\\'])
            .all(|part| !part.is_empty() && part != ".." && part != ".")
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
}
