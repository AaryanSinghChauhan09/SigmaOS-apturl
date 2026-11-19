// SPDX-License-Identifier: MIT OR Apache-2.0
//! SigmaOS-apturl — apt:// URL handler.
//!
//! Provides parsing, validation, and resolution for `apt:` and `apt+http:`
//! uniform resource identifiers across SigmaOS desktops.
//!
//! Inspired by Omarchy's minimalist protocol handling, Hyprland's strict IPC guarantees,
//! and the Catppuccin Mocha aesthetic of calm, structured clarity.
//!
//! Guarantees: strict URL syntax checking, query parameter isolation, pure `std`, zero `unsafe`.

#![forbid(unsafe_code)]

use std::collections::HashMap;
use std::fmt;

// ── Error types ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AptUrlError {
    InvalidScheme(String),
    EmptyPackageList,
    InvalidPackageName(String),
    InvalidParameter(String),
    MalformedUrl(String),
}

impl fmt::Display for AptUrlError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidScheme(s) => write!(f, "unsupported scheme: '{s}', expected 'apt' or 'apt+http'"),
            Self::EmptyPackageList => write!(f, "url contains no package targets"),
            Self::InvalidPackageName(pkg) => write!(f, "invalid Debian package name: '{pkg}'"),
            Self::InvalidParameter(p) => write!(f, "invalid query parameter: '{p}'"),
            Self::MalformedUrl(u) => write!(f, "malformed URL syntax: '{u}'"),
        }
    }
}

impl std::error::Error for AptUrlError {}

// ── Core Domain Types ────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstallAction {
    Install,
    Upgrade,
    Reinstall,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AptPackageRequest {
    pub name: String,
    pub target_version: Option<String>,
}

impl AptPackageRequest {
    pub fn new(name: impl Into<String>) -> Result<Self, AptUrlError> {
        let name = name.into().trim().to_lowercase();
        if !is_valid_debian_pkg_name(&name) {
            return Err(AptUrlError::InvalidPackageName(name));
        }
        Ok(Self {
            name,
            target_version: None,
        })
    }

    pub fn with_version(mut self, ver: impl Into<String>) -> Self {
        self.target_version = Some(ver.into());
        self
    }
}

fn is_valid_debian_pkg_name(name: &str) -> bool {
    if name.is_empty() || name.len() > 128 {
        return false;
    }
    let first = name.chars().next().unwrap();
    if !first.is_ascii_alphanumeric() {
        return false;
    }
    name.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '+' || c == '-' || c == '.')
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AptUrlRequest {
    pub action: InstallAction,
    pub packages: Vec<AptPackageRequest>,
    pub channel: Option<String>,
    pub auto_confirm: bool,
    pub extra_params: HashMap<String, String>,
}

impl AptUrlRequest {
    pub fn parse(raw_url: &str) -> Result<Self, AptUrlError> {
        let trimmed = raw_url.trim();
        if trimmed.is_empty() {
            return Err(AptUrlError::MalformedUrl("empty url".into()));
        }

        let (scheme, rest) = match trimmed.split_once(':') {
            Some((s, r)) => (s.to_lowercase(), r),
            None => return Err(AptUrlError::MalformedUrl("missing scheme delimiter".into())),
        };

        if scheme != "apt" && scheme != "apt+http" && scheme != "apt+https" {
            return Err(AptUrlError::InvalidScheme(scheme));
        }

        let body = rest.trim_start_matches('/');
        let (path_part, query_part) = match body.split_once('?') {
            Some((p, q)) => (p, Some(q)),
            None => (body, None),
        };

        if path_part.is_empty() {
            return Err(AptUrlError::EmptyPackageList);
        }

        let mut packages = Vec::new();
        for item in path_part.split(',') {
            let item = item.trim();
            if item.is_empty() {
                continue;
            }
            if let Some((pkg, ver)) = item.split_once('=') {
                packages.push(AptPackageRequest::new(pkg)?.with_version(ver));
            } else {
                packages.push(AptPackageRequest::new(item)?);
            }
        }

        if packages.is_empty() {
            return Err(AptUrlError::EmptyPackageList);
        }

        let mut channel = None;
        let mut auto_confirm = false;
        let mut action = InstallAction::Install;
        let mut extra_params = HashMap::new();

        if let Some(query) = query_part {
            for pair in query.split('&') {
                if pair.is_empty() {
                    continue;
                }
                let (k, v) = match pair.split_once('=') {
                    Some((k, v)) => (k.trim().to_lowercase(), v.trim().to_string()),
                    None => (pair.trim().to_lowercase(), String::new()),
                };

                match k.as_str() {
                    "channel" => channel = Some(v),
                    "action" => match v.as_str() {
                        "upgrade" => action = InstallAction::Upgrade,
                        "reinstall" => action = InstallAction::Reinstall,
                        "install" | "" => action = InstallAction::Install,
                        other => return Err(AptUrlError::InvalidParameter(format!("unknown action '{other}'"))),
                    },
                    "confirm" => auto_confirm = v == "auto" || v == "1" || v == "true",
                    other => {
                        extra_params.insert(other.to_string(), v);
                    }
                }
            }
        }

        Ok(Self {
            action,
            packages,
            channel,
            auto_confirm,
            extra_params,
        })
    }

    pub fn to_apt_command(&self) -> Vec<String> {
        let mut cmd = vec!["apt-get".to_string()];
        match self.action {
            InstallAction::Install => cmd.push("install".to_string()),
            InstallAction::Upgrade => cmd.push("install".to_string()),
            InstallAction::Reinstall => {
                cmd.push("install".to_string());
                cmd.push("--reinstall".to_string());
            }
        }
        if self.auto_confirm {
            cmd.push("-y".to_string());
        }
        for pkg in &self.packages {
            if let Some(ref ver) = pkg.target_version {
                cmd.push(format!("{}={}", pkg.name, ver));
            } else {
                cmd.push(pkg.name.clone());
            }
        }
        cmd
    }
}

// ── Builder ──────────────────────────────────────────────────────────────────

#[derive(Debug, Default)]
pub struct AptUrlBuilder {
    packages: Vec<AptPackageRequest>,
    action: Option<InstallAction>,
    channel: Option<String>,
    auto_confirm: bool,
}

impl AptUrlBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add_package(mut self, pkg: impl Into<String>) -> Result<Self, AptUrlError> {
        self.packages.push(AptPackageRequest::new(pkg)?);
        Ok(self)
    }

    pub fn action(mut self, action: InstallAction) -> Self {
        self.action = Some(action);
        self
    }

    pub fn channel(mut self, ch: impl Into<String>) -> Self {
        self.channel = Some(ch.into());
        self
    }

    pub fn auto_confirm(mut self, yes: bool) -> Self {
        self.auto_confirm = yes;
        self
    }

    pub fn build(self) -> Result<AptUrlRequest, AptUrlError> {
        if self.packages.is_empty() {
            return Err(AptUrlError::EmptyPackageList);
        }
        Ok(AptUrlRequest {
            action: self.action.unwrap_or(InstallAction::Install),
            packages: self.packages,
            channel: self.channel,
            auto_confirm: self.auto_confirm,
            extra_params: HashMap::new(),
        })
    }
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_valid_single_package() {
        // Arrange
        let url = "apt:ripgrep";
        // Act
        let req = AptUrlRequest::parse(url).unwrap();
        // Assert
        assert_eq!(req.packages.len(), 1);
        assert_eq!(req.packages[0].name, "ripgrep");
        assert_eq!(req.action, InstallAction::Install);
        assert!(!req.auto_confirm);
    }

    #[test]
    fn parse_multiple_packages_with_versions() {
        // Arrange
        let url = "apt://neovim=0.9.5,kitty?action=upgrade&confirm=true";
        // Act
        let req = AptUrlRequest::parse(url).unwrap();
        // Assert
        assert_eq!(req.packages.len(), 2);
        assert_eq!(req.packages[0].name, "neovim");
        assert_eq!(req.packages[0].target_version, Some("0.9.5".into()));
        assert_eq!(req.packages[1].name, "kitty");
        assert_eq!(req.action, InstallAction::Upgrade);
        assert!(req.auto_confirm);
    }

    #[test]
    fn reject_invalid_scheme() {
        // Arrange
        let url = "http://example.com/foo";
        // Act
        let err = AptUrlRequest::parse(url).unwrap_err();
        // Assert
        assert!(matches!(err, AptUrlError::InvalidScheme(_)));
    }

    #[test]
    fn reject_empty_packages() {
        // Arrange
        let url = "apt:";
        // Act
        let err = AptUrlRequest::parse(url).unwrap_err();
        // Assert
        assert_eq!(err, AptUrlError::EmptyPackageList);
    }

    #[test]
    fn reject_invalid_package_name_characters() {
        // Arrange
        let url = "apt:malicious;rm -rf /";
        // Act
        let err = AptUrlRequest::parse(url).unwrap_err();
        // Assert
        assert!(matches!(err, AptUrlError::InvalidPackageName(_)));
    }

    #[test]
    fn builder_creates_valid_request() {
        // Arrange
        let builder = AptUrlBuilder::new()
            .add_package("tmux").unwrap()
            .action(InstallAction::Reinstall)
            .auto_confirm(true);
        // Act
        let req = builder.build().unwrap();
        // Assert
        assert_eq!(req.packages[0].name, "tmux");
        assert_eq!(req.action, InstallAction::Reinstall);
        assert!(req.auto_confirm);
    }

    #[test]
    fn to_apt_command_generation() {
        // Arrange
        let req = AptUrlBuilder::new()
            .add_package("btop").unwrap()
            .auto_confirm(true)
            .build()
            .unwrap();
        // Act
        let cmd = req.to_apt_command();
        // Assert
        assert_eq!(cmd, vec!["apt-get", "install", "-y", "btop"]);
    }

    #[test]
    fn error_display_formats() {
        // Arrange
        let err = AptUrlError::InvalidPackageName("foo@bar".into());
        // Act & Assert
        assert_eq!(format!("{err}"), "invalid Debian package name: 'foo@bar'");
    }
}
