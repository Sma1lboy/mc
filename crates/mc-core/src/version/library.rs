//! Library entries and resolution of which jar / native applies on this platform.

use std::collections::BTreeMap;

use serde::Deserialize;

use mc_types::Os;

use super::gradle::GradleSpec;
use super::rule::{rules_allow, Rule, RuntimeContext};

/// A single downloadable file with integrity metadata.
#[derive(Debug, Clone, Deserialize)]
pub struct Artifact {
    /// Repository-relative path. Optional for `url`-only maven libraries.
    #[serde(default)]
    pub path: Option<String>,
    pub url: String,
    #[serde(default)]
    pub sha1: Option<String>,
    #[serde(default)]
    pub size: Option<u64>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct LibraryDownloads {
    #[serde(default)]
    pub artifact: Option<Artifact>,
    /// Old-style platform natives, keyed by classifier (e.g. "natives-windows").
    #[serde(default)]
    pub classifiers: Option<BTreeMap<String, Artifact>>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct ExtractRules {
    #[serde(default)]
    pub exclude: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Library {
    pub name: String,
    #[serde(default)]
    pub downloads: Option<LibraryDownloads>,
    #[serde(default)]
    pub rules: Vec<Rule>,
    /// Old-style native map: OS name → classifier template (may contain `${arch}`).
    #[serde(default)]
    pub natives: Option<BTreeMap<String, String>>,
    /// Maven base URL for `url`-only libraries (Forge/Fabric style).
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub extract: Option<ExtractRules>,
}

/// A resolved file to download/place: where it goes, where it comes from, its hash.
#[derive(Debug, Clone)]
pub struct ResolvedFile {
    /// Repository-relative path (under `libraries/`).
    pub path: String,
    pub url: String,
    pub sha1: Option<String>,
    pub size: Option<u64>,
    /// True when this file must be extracted into the natives directory.
    pub is_native: bool,
    pub extract_exclude: Vec<String>,
}

impl Library {
    /// The parsed coordinate.
    pub fn spec(&self) -> Option<GradleSpec> {
        GradleSpec::parse(&self.name)
    }

    /// Is this library enabled on the given platform?
    pub fn applies(&self, ctx: &RuntimeContext) -> bool {
        rules_allow(&self.rules, ctx)
    }

    /// Resolve the classpath jar for this library (the main artifact), if any.
    pub fn classpath_file(&self, default_maven: &str) -> Option<ResolvedFile> {
        let spec = self.spec()?;

        // Prefer the explicit artifact download block.
        if let Some(dl) = &self.downloads {
            if let Some(art) = &dl.artifact {
                let path = art.path.clone().unwrap_or_else(|| spec.to_url_path());
                return Some(ResolvedFile {
                    path,
                    url: art.url.clone(),
                    sha1: art.sha1.clone(),
                    size: art.size,
                    is_native: false,
                    extract_exclude: vec![],
                });
            }
        }

        // `url`-only maven library (or no download info): build the URL from the coord.
        let base = self.url.clone().unwrap_or_else(|| default_maven.to_string());
        let rel = spec.to_url_path();
        let url = format!("{}/{}", base.trim_end_matches('/'), rel);
        Some(ResolvedFile {
            path: rel,
            url,
            sha1: None,
            size: None,
            is_native: false,
            extract_exclude: vec![],
        })
    }

    /// Resolve the native jar for the current OS, handling both the old `natives`
    /// map style and the new dedicated-classifier style.
    pub fn native_file(&self, ctx: &RuntimeContext) -> Option<ResolvedFile> {
        let spec = self.spec()?;
        let exclude = self.extract.as_ref().map(|e| e.exclude.clone()).unwrap_or_default();

        // Old style: `natives` map → classifier key, looked up in `downloads.classifiers`.
        if let Some(natives) = &self.natives {
            let key_template = natives.get(ctx.platform.os.mojang_name())?;
            let classifier = key_template.replace("${arch}", arch_bits(ctx.platform.os));
            if let Some(dl) = &self.downloads {
                if let Some(classifiers) = &dl.classifiers {
                    if let Some(art) = classifiers.get(&classifier) {
                        let path = art.path.clone().unwrap_or_else(|| spec.with_classifier(&classifier).to_url_path());
                        return Some(ResolvedFile {
                            path,
                            url: art.url.clone(),
                            sha1: art.sha1.clone(),
                            size: art.size,
                            is_native: true,
                            extract_exclude: exclude,
                        });
                    }
                }
            }
            // No download block: synthesise from coordinate.
            let nspec = spec.with_classifier(&classifier);
            let base = self.url.clone().unwrap_or_else(|| "https://libraries.minecraft.net".to_string());
            let rel = nspec.to_url_path();
            return Some(ResolvedFile {
                path: rel.clone(),
                url: format!("{}/{}", base.trim_end_matches('/'), rel),
                sha1: None,
                size: None,
                is_native: true,
                extract_exclude: exclude,
            });
        }

        None
    }
}

/// `${arch}` substitution value for the old-style native map (32/64).
fn arch_bits(_os: Os) -> &'static str {
    // Modern launchers run 64-bit; the legacy templates only ever asked for 32/64.
    "64"
}

impl Library {
    /// New-style native classifier (e.g. `natives-macos-arm64`) when this entry
    /// IS itself a native bundle (1.19+ ships natives as separate library entries
    /// with a `natives-*` classifier baked into the coordinate).
    pub fn native_classifier(&self) -> Option<String> {
        let c = self.spec()?.classifier?;
        if c.starts_with("natives") {
            Some(c)
        } else {
            None
        }
    }
}

/// Among the new-style native library entries, pick the single best one per base
/// artifact for the running architecture.
///
/// Mojang's `osx` rule matches both `natives-macos` and `natives-macos-arm64` on
/// an Apple-silicon machine, so the rule check alone is ambiguous: we additionally
/// prefer the arch-specific classifier (arm64 on arm, the non-arm one on x64).
pub fn select_native_libraries<'a>(libs: &'a [Library], ctx: &RuntimeContext) -> Vec<&'a Library> {
    use std::collections::BTreeMap;
    use mc_types::Arch;

    let mut groups: BTreeMap<String, Vec<&Library>> = BTreeMap::new();
    for lib in libs {
        if lib.native_classifier().is_none() || !lib.applies(ctx) {
            continue;
        }
        let key = lib
            .spec()
            .map(|s| format!("{}:{}", s.group, s.artifact))
            .unwrap_or_default();
        groups.entry(key).or_default().push(lib);
    }

    let want_arm = ctx.platform.arch == Arch::Arm64;
    let mut out = Vec::new();
    for (_, cands) in groups {
        let is_arm = |l: &&Library| {
            let c = l.native_classifier().unwrap_or_default();
            c.contains("arm64") || c.contains("aarch64")
        };
        let arm = cands.iter().find(|l| is_arm(l)).copied();
        let other = cands.iter().find(|l| !is_arm(l)).copied();
        let chosen = if want_arm { arm.or(other) } else { other.or(arm) };
        if let Some(c) = chosen {
            out.push(c);
        }
    }
    out
}

/// The classpath libraries: everything that applies and is NOT a native bundle.
pub fn classpath_libraries<'a>(libs: &'a [Library], ctx: &RuntimeContext) -> Vec<&'a Library> {
    libs.iter()
        .filter(|l| l.applies(ctx) && l.native_classifier().is_none())
        .collect()
}
