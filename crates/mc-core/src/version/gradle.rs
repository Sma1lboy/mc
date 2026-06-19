//! Gradle/Maven artifact coordinates and their conversion to repository paths.
//!
//! A coordinate looks like `group:artifact:version[:classifier][@ext]`, e.g.
//! `org.lwjgl:lwjgl:3.3.1:natives-windows`.

use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GradleSpec {
    pub group: String,
    pub artifact: String,
    pub version: String,
    pub classifier: Option<String>,
    pub extension: String,
}

impl GradleSpec {
    /// Parse a coordinate string. Returns `None` if it has fewer than the three
    /// mandatory `group:artifact:version` components.
    pub fn parse(s: &str) -> Option<GradleSpec> {
        // Split off the optional `@ext` suffix first.
        let (coord, extension) = match s.rsplit_once('@') {
            Some((c, e)) if !e.contains(':') => (c, e.to_string()),
            _ => (s, "jar".to_string()),
        };

        let mut parts = coord.split(':');
        let group = parts.next()?.to_string();
        let artifact = parts.next()?.to_string();
        let version = parts.next()?.to_string();
        let classifier = parts.next().map(|c| c.to_string());

        if group.is_empty() || artifact.is_empty() || version.is_empty() {
            return None;
        }

        Some(GradleSpec { group, artifact, version, classifier, extension })
    }

    /// Relative repository path:
    /// `group(dots→slashes)/artifact/version/artifact-version[-classifier].ext`
    pub fn to_path(&self) -> PathBuf {
        let group_path = self.group.replace('.', "/");
        let file = match &self.classifier {
            Some(c) => format!("{}-{}-{}.{}", self.artifact, self.version, c, self.extension),
            None => format!("{}-{}.{}", self.artifact, self.version, self.extension),
        };
        PathBuf::from(group_path)
            .join(&self.artifact)
            .join(&self.version)
            .join(file)
    }

    /// Same as [`to_path`] but as a forward-slash string suitable for URLs.
    pub fn to_url_path(&self) -> String {
        self.to_path().to_string_lossy().replace('\\', "/")
    }

    /// A new spec with the given classifier (used to resolve natives).
    pub fn with_classifier(&self, classifier: &str) -> GradleSpec {
        GradleSpec { classifier: Some(classifier.to_string()), ..self.clone() }
    }
}

impl std::fmt::Display for GradleSpec {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}:{}:{}", self.group, self.artifact, self.version)?;
        if let Some(c) = &self.classifier {
            write!(f, ":{c}")?;
        }
        if self.extension != "jar" {
            write!(f, "@{}", self.extension)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_basic() {
        let s = GradleSpec::parse("org.lwjgl:lwjgl:3.3.1").unwrap();
        assert_eq!(s.group, "org.lwjgl");
        assert_eq!(s.artifact, "lwjgl");
        assert_eq!(s.version, "3.3.1");
        assert_eq!(s.classifier, None);
        assert_eq!(s.extension, "jar");
        assert_eq!(s.to_url_path(), "org/lwjgl/lwjgl/3.3.1/lwjgl-3.3.1.jar");
    }

    #[test]
    fn parses_classifier() {
        let s = GradleSpec::parse("org.lwjgl:lwjgl:3.3.1:natives-windows").unwrap();
        assert_eq!(s.classifier.as_deref(), Some("natives-windows"));
        assert_eq!(
            s.to_url_path(),
            "org/lwjgl/lwjgl/3.3.1/lwjgl-3.3.1-natives-windows.jar"
        );
    }

    #[test]
    fn parses_extension() {
        let s = GradleSpec::parse("net.minecraftforge:forge:1.20.1-47.2.0:installer@jar").unwrap();
        assert_eq!(s.extension, "jar");
        assert_eq!(s.classifier.as_deref(), Some("installer"));
    }

    #[test]
    fn rejects_incomplete() {
        assert!(GradleSpec::parse("group:artifact").is_none());
    }
}
