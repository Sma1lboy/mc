//! List available mod-loader build numbers for a given Minecraft version.
//!
//! Powers the New Instance dialog's loader-version picker so users don't have to
//! hand-type exact build numbers like "47.2.0". Each loader exposes a different
//! source:
//!
//! - **Forge / NeoForge** publish a Maven `maven-metadata.xml` listing every
//!   build. Forge versions look like `1.20.1-47.2.0`; we filter to the requested
//!   `mc_version` prefix and return the build half. NeoForge versions look like
//!   `20.4.237`; we keep only those whose derived MC version matches.
//! - **Fabric / Quilt** expose a JSON meta API of loader versions (MC-agnostic);
//!   we return the raw loader version strings, newest first.
//!
//! All sources list results newest-first already (Maven is reversed here), so the
//! caller can preselect the first entry as the recommended default. Any failure
//! (network, unsupported loader) bubbles up as a [`CoreError`]; the command layer
//! turns that into an empty list so the UI can fall back to a free-text input
//! rather than blocking the user.

use mc_types::LoaderKind;

use crate::download::Downloader;
use crate::error::Result;

use super::neoforge;

const FORGE_METADATA: &str =
    "https://maven.minecraftforge.net/net/minecraftforge/forge/maven-metadata.xml";
const NEOFORGE_METADATA: &str =
    "https://maven.neoforged.net/releases/net/neoforged/neoforge/maven-metadata.xml";
const FABRIC_LOADERS: &str = "https://meta.fabricmc.net/v2/versions/loader";
const QUILT_LOADERS: &str = "https://meta.quiltmc.org/v3/versions/loader";

/// List the available loader build numbers for `loader` + `mc_version`, newest
/// first. Returns an empty vec for loaders without a version list (Vanilla and
/// loaders that have no independent installer here, e.g. LiteLoader/OptiFine).
///
/// The returned strings are the **bare build numbers** the installers expect:
/// `47.2.0` for Forge, `20.4.237` for NeoForge, `0.16.5` for Fabric/Quilt.
pub async fn list_loader_versions(
    dl: &Downloader,
    loader: LoaderKind,
    mc_version: &str,
) -> Result<Vec<String>> {
    match loader {
        LoaderKind::Forge => {
            let xml = dl.get_text(FORGE_METADATA).await?;
            Ok(parse_forge_versions(&xml, mc_version))
        }
        LoaderKind::NeoForge => {
            let xml = dl.get_text(NEOFORGE_METADATA).await?;
            Ok(parse_neoforge_versions(&xml, mc_version))
        }
        LoaderKind::Fabric => {
            let json = dl.get_text(FABRIC_LOADERS).await?;
            Ok(parse_meta_loader_versions(&json))
        }
        LoaderKind::Quilt => {
            let json = dl.get_text(QUILT_LOADERS).await?;
            Ok(parse_meta_loader_versions(&json))
        }
        LoaderKind::Vanilla | LoaderKind::LiteLoader | LoaderKind::OptiFine => Ok(Vec::new()),
    }
}

/// Pull the text of every `<version>…</version>` element out of a Maven
/// `maven-metadata.xml`. The file is machine-generated and flat, so a tag scan
/// is sufficient and avoids pulling in an XML dependency.
fn maven_versions(xml: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut rest = xml;
    while let Some(start) = rest.find("<version>") {
        let after = &rest[start + "<version>".len()..];
        match after.find("</version>") {
            Some(end) => {
                let v = after[..end].trim();
                if !v.is_empty() {
                    out.push(v);
                }
                rest = &after[end + "</version>".len()..];
            }
            None => break,
        }
    }
    out
}

/// Keep Forge versions matching `mc_version` and return the build half, newest
/// first. Forge maven entries look like `1.20.1-47.2.0`; some legacy entries
/// carry extra suffixes (`1.7.10-10.13.4.1614-1.7.10`) — we keep everything
/// after the `<mc>-` prefix verbatim so the installer URL round-trips.
fn parse_forge_versions(xml: &str, mc_version: &str) -> Vec<String> {
    let prefix = format!("{mc_version}-");
    let mut builds: Vec<String> = maven_versions(xml)
        .into_iter()
        .filter_map(|v| v.strip_prefix(&prefix))
        .filter(|b| !b.is_empty())
        .map(str::to_string)
        .collect();
    // Maven lists oldest-first; newest first is friendlier as a default.
    builds.reverse();
    builds
}

/// Keep NeoForge versions whose derived MC version matches `mc_version`, newest
/// first. NeoForge maven entries are bare (`20.4.237`).
fn parse_neoforge_versions(xml: &str, mc_version: &str) -> Vec<String> {
    let mut builds: Vec<String> = maven_versions(xml)
        .into_iter()
        .filter(|v| neoforge::mc_version_for(v).as_deref() == Some(mc_version))
        .map(str::to_string)
        .collect();
    builds.reverse();
    builds
}

/// Parse the Fabric/Quilt meta loader list, which is a JSON array of objects
/// shaped `{ "loader": { "version": "0.16.5", … }, … }`, already newest-first.
fn parse_meta_loader_versions(json: &str) -> Vec<String> {
    #[derive(serde::Deserialize)]
    struct Entry {
        loader: Loader,
    }
    #[derive(serde::Deserialize)]
    struct Loader {
        version: String,
    }
    serde_json::from_str::<Vec<Entry>>(json)
        .map(|list| list.into_iter().map(|e| e.loader.version).collect())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    const FORGE_XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<metadata>
  <groupId>net.minecraftforge</groupId>
  <artifactId>forge</artifactId>
  <versioning>
    <release>1.20.1-47.2.0</release>
    <versions>
      <version>1.19.2-43.1.1</version>
      <version>1.20.1-47.1.0</version>
      <version>1.20.1-47.1.3</version>
      <version>1.20.1-47.2.0</version>
      <version>1.7.10-10.13.4.1614-1.7.10</version>
    </versions>
  </versioning>
</metadata>"#;

    const NEOFORGE_XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<metadata>
  <versioning>
    <versions>
      <version>20.2.88</version>
      <version>20.4.80</version>
      <version>20.4.190</version>
      <version>20.4.237</version>
      <version>21.0.0-beta</version>
    </versions>
  </versioning>
</metadata>"#;

    const FABRIC_JSON: &str = r#"[
      {"loader": {"separator": ".", "build": 1, "maven": "x", "version": "0.16.5", "stable": true}},
      {"loader": {"separator": ".", "build": 1, "maven": "x", "version": "0.16.4", "stable": true}},
      {"loader": {"separator": ".", "build": 1, "maven": "x", "version": "0.15.11", "stable": true}}
    ]"#;

    #[test]
    fn forge_filters_to_mc_version_and_returns_build_newest_first() {
        let v = parse_forge_versions(FORGE_XML, "1.20.1");
        assert_eq!(v, vec!["47.2.0", "47.1.3", "47.1.0"]);
    }

    #[test]
    fn forge_returns_empty_for_unknown_mc_version() {
        assert!(parse_forge_versions(FORGE_XML, "1.21.4").is_empty());
    }

    #[test]
    fn neoforge_keeps_only_matching_mc_version_newest_first() {
        let v = parse_neoforge_versions(NEOFORGE_XML, "1.20.4");
        // 21.0.0-beta -> mc 1.21.0 (excluded); 20.2.88 -> 1.20.2 (excluded).
        assert_eq!(v, vec!["20.4.237", "20.4.190", "20.4.80"]);
    }

    #[test]
    fn neoforge_returns_empty_for_unknown_mc_version() {
        assert!(parse_neoforge_versions(NEOFORGE_XML, "1.19.4").is_empty());
    }

    #[test]
    fn meta_loaders_return_versions_in_source_order() {
        let v = parse_meta_loader_versions(FABRIC_JSON);
        assert_eq!(v, vec!["0.16.5", "0.16.4", "0.15.11"]);
    }

    #[test]
    fn maven_versions_handles_malformed_input_gracefully() {
        // Unterminated tag → stop scanning, return what we have.
        assert_eq!(maven_versions("<version>1.0</version><version>broken"), vec!["1.0"]);
        assert!(maven_versions("no tags here").is_empty());
    }
}
