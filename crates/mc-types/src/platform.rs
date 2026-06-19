//! Operating-system and CPU-architecture identification, in the vocabulary
//! Mojang's version json uses for rule evaluation and native selection.

use serde::{Deserialize, Serialize};

/// Operating system, named the way Mojang rules name them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Os {
    Windows,
    /// Mojang calls macOS "osx".
    #[serde(rename = "osx")]
    MacOs,
    Linux,
}

impl Os {
    /// The string Mojang uses in `rules[].os.name`.
    pub fn mojang_name(self) -> &'static str {
        match self {
            Os::Windows => "windows",
            Os::MacOs => "osx",
            Os::Linux => "linux",
        }
    }

    /// The current host OS, resolved at compile time.
    pub const fn current() -> Os {
        #[cfg(target_os = "windows")]
        {
            Os::Windows
        }
        #[cfg(target_os = "macos")]
        {
            Os::MacOs
        }
        #[cfg(not(any(target_os = "windows", target_os = "macos")))]
        {
            Os::Linux
        }
    }

    /// Classpath / library-path separator for this OS (`;` on Windows, `:` elsewhere).
    pub fn classpath_separator(self) -> char {
        match self {
            Os::Windows => ';',
            _ => ':',
        }
    }
}

/// CPU architecture, named the way Mojang rules name them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Arch {
    X86,
    X64,
    Arm64,
    Arm32,
}

impl Arch {
    /// The string Mojang uses in `rules[].os.arch` (e.g. "x86", "arm64").
    pub fn mojang_name(self) -> &'static str {
        match self {
            Arch::X86 => "x86",
            Arch::X64 => "x64",
            Arch::Arm64 => "arm64",
            Arch::Arm32 => "arm32",
        }
    }

    pub const fn current() -> Arch {
        #[cfg(target_arch = "x86_64")]
        {
            Arch::X64
        }
        #[cfg(target_arch = "x86")]
        {
            Arch::X86
        }
        #[cfg(target_arch = "aarch64")]
        {
            Arch::Arm64
        }
        #[cfg(target_arch = "arm")]
        {
            Arch::Arm32
        }
        #[cfg(not(any(
            target_arch = "x86_64",
            target_arch = "x86",
            target_arch = "aarch64",
            target_arch = "arm"
        )))]
        {
            Arch::X64
        }
    }
}

/// The resolved host platform.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Platform {
    pub os: Os,
    pub arch: Arch,
}

impl Platform {
    pub const fn current() -> Platform {
        Platform { os: Os::current(), arch: Arch::current() }
    }
}

impl Default for Platform {
    fn default() -> Self {
        Platform::current()
    }
}
