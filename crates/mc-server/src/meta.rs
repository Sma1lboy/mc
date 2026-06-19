//! Loader/version meta aggregation. The launcher hits one endpoint instead of
//! four upstreams; each upstream failure degrades to an empty list rather than
//! failing the whole response.

use serde::Serialize;

const FABRIC_META: &str = "https://meta.fabricmc.net/v2";
const QUILT_META: &str = "https://meta.quiltmc.org/v3";
const NEOFORGE_MAVEN: &str =
    "https://maven.neoforged.net/api/maven/versions/releases/net/neoforged/neoforge";
const FORGE_PROMOS: &str =
    "https://files.minecraftforge.net/net/minecraftforge/forge/promotions_slim.json";
const MOJANG_MANIFEST: &str =
    "https://piston-meta.mojang.com/mc/game/version_manifest_v2.json";

#[derive(Serialize)]
pub struct LoaderMeta {
    pub mc_version: String,
    pub loaders: Loaders,
}

#[derive(Serialize, Default)]
pub struct Loaders {
    pub fabric: Vec<String>,
    pub quilt: Vec<String>,
    pub forge: Vec<String>,
    pub neoforge: Vec<String>,
}

#[derive(Serialize)]
pub struct VersionEntry {
    pub id: String,
    pub kind: String,
    pub release_time: String,
}

/// Aggregate all loader families' versions available for `mc_version`.
pub async fn loaders_for(http: &reqwest::Client, mc_version: &str) -> LoaderMeta {
    // Run the four upstream lookups concurrently.
    let fabric_url = format!("{FABRIC_META}/versions/loader/{mc_version}");
    let quilt_url = format!("{QUILT_META}/versions/loader/{mc_version}");
    let (fabric, quilt, forge, neoforge) = tokio::join!(
        fabric_like(http, &fabric_url),
        fabric_like(http, &quilt_url),
        forge_versions(http, mc_version),
        neoforge_versions(http, mc_version),
    );
    LoaderMeta {
        mc_version: mc_version.to_string(),
        loaders: Loaders { fabric, quilt, forge, neoforge },
    }
}

/// Fabric and Quilt share the same `[{loader:{version}}]` response shape.
async fn fabric_like(http: &reqwest::Client, url: &str) -> Vec<String> {
    let Ok(resp) = http.get(url).send().await else { return vec![] };
    let Ok(json) = resp.json::<serde_json::Value>().await else { return vec![] };
    json.as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|e| e.get("loader").and_then(|l| l.get("version")).and_then(|v| v.as_str()))
                .map(|s| s.to_string())
                .collect()
        })
        .unwrap_or_default()
}

/// Forge: the promotions feed exposes latest/recommended per mc version.
async fn forge_versions(http: &reqwest::Client, mc_version: &str) -> Vec<String> {
    let Ok(resp) = http.get(FORGE_PROMOS).send().await else { return vec![] };
    let Ok(json) = resp.json::<serde_json::Value>().await else { return vec![] };
    let Some(promos) = json.get("promos").and_then(|p| p.as_object()) else { return vec![] };

    let mut out = Vec::new();
    // Keys look like "1.20.1-latest" / "1.20.1-recommended".
    for suffix in ["recommended", "latest"] {
        let key = format!("{mc_version}-{suffix}");
        if let Some(v) = promos.get(&key).and_then(|v| v.as_str()) {
            let full = format!("{mc_version}-{v}");
            if !out.contains(&full) {
                out.push(full);
            }
        }
    }
    out
}

/// NeoForge: versions for 1.X.Y are prefixed "X.Y." (e.g. 1.20.4 -> 20.4.*).
async fn neoforge_versions(http: &reqwest::Client, mc_version: &str) -> Vec<String> {
    let Some(prefix) = neoforge_prefix(mc_version) else { return vec![] };
    let Ok(resp) = http.get(NEOFORGE_MAVEN).send().await else { return vec![] };
    let Ok(json) = resp.json::<serde_json::Value>().await else { return vec![] };
    let mut out: Vec<String> = json
        .get("versions")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str())
                .filter(|v| v.starts_with(&prefix))
                .map(|s| s.to_string())
                .collect()
        })
        .unwrap_or_default();
    // newest first
    out.reverse();
    out.truncate(30);
    out
}

/// "1.20.4" -> "20.4." ; "1.21" -> "21.0."
fn neoforge_prefix(mc_version: &str) -> Option<String> {
    let mut parts = mc_version.strip_prefix("1.")?.split('.');
    let minor = parts.next()?;
    let patch = parts.next().unwrap_or("0");
    Some(format!("{minor}.{patch}."))
}

/// Pass-through of Mojang's version manifest, normalised to a slim shape.
pub async fn versions(http: &reqwest::Client) -> Vec<VersionEntry> {
    let Ok(resp) = http.get(MOJANG_MANIFEST).send().await else { return vec![] };
    let Ok(json) = resp.json::<serde_json::Value>().await else { return vec![] };
    json.get("versions")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| {
                    Some(VersionEntry {
                        id: v.get("id")?.as_str()?.to_string(),
                        kind: v.get("type")?.as_str()?.to_string(),
                        release_time: v
                            .get("releaseTime")
                            .and_then(|t| t.as_str())
                            .unwrap_or("")
                            .to_string(),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn neoforge_prefix_maps() {
        assert_eq!(neoforge_prefix("1.20.4").as_deref(), Some("20.4."));
        assert_eq!(neoforge_prefix("1.21").as_deref(), Some("21.0."));
        assert_eq!(neoforge_prefix("garbage"), None);
    }
}
