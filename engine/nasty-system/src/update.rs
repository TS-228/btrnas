//! System updates on Debian: apt package upgrades with snapper (btrfs)
//! snapshots for generations / rollback.
//!
//! Generations are snapper snapshots on the `root` config. apt hooks
//! shipped with the `nasty` package create pre/post snapshots around
//! package changes; `apply` also takes an explicit pre snapshot.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use std::time::Duration;
use thiserror::Error;
use tracing::{info, warn};

const VERSION_PATH: &str = "/var/lib/nasty/version";
const VERSION_PATH_FALLBACK: &str = "/etc/nasty-version";
const UPDATE_UNIT: &str = "nasty-update";
const UPDATE_WEBUI_CHANGED: &str = "/var/lib/nasty/update-webui-changed";
const RELEASE_CHANNEL_PATH: &str = "/var/lib/nasty/release-channel";
const UPDATE_BUILD_DIR_PATH: &str = "/var/lib/nasty/update-build-dir";
const GENERATION_LABELS_PATH: &str = "/var/lib/nasty/generation-labels.json";
const LAST_ATTEMPT_PATH: &str = "/var/lib/nasty/last-upgrade-attempt";
const SNAPPER_CONFIG: &str = "root";
/// Binary package whose version is the appliance "NASty version".
const NASTY_METAPACKAGE: &str = "nasty";
const NASTY_ENGINE_PKG: &str = "nasty-engine";
const NASTY_WEBUI_PKG: &str = "nasty-webui";

const GITHUB_FETCH_TIMEOUT: Duration = Duration::from_secs(60);
const DEFAULT_NASTY_OWNER: &str = "nasty-project";
const DEFAULT_NASTY_REPO: &str = "nasty";

#[derive(Debug, Deserialize)]
struct GitHubRelease {
    tag_name: String,
    draft: bool,
    #[allow(dead_code)]
    prerelease: bool,
}

// ── Release channels ────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum ReleaseChannel {
    /// Tagged releases only (v*).
    Mild,
    /// Pre-release tags (s*).
    Spicy,
    /// Track apt candidate / bleeding packages.
    Nasty,
}

impl ReleaseChannel {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Mild => "mild",
            Self::Spicy => "spicy",
            Self::Nasty => "nasty",
        }
    }

    pub fn display_name(self) -> &'static str {
        match self {
            Self::Mild => "Mild",
            Self::Spicy => "Spicy",
            Self::Nasty => "Nasty",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "mild" => Some(Self::Mild),
            "spicy" => Some(Self::Spicy),
            "nasty" => Some(Self::Nasty),
            _ => None,
        }
    }
}

impl std::str::FromStr for ReleaseChannel {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::parse(s).ok_or_else(|| format!("unknown channel: {s}"))
    }
}

pub async fn read_channel() -> ReleaseChannel {
    match tokio::fs::read_to_string(RELEASE_CHANNEL_PATH).await {
        Ok(s) => ReleaseChannel::parse(&s).unwrap_or(ReleaseChannel::Mild),
        Err(_) => ReleaseChannel::Mild,
    }
}

pub async fn read_update_build_dir() -> Option<String> {
    match tokio::fs::read_to_string(UPDATE_BUILD_DIR_PATH).await {
        Ok(s) => {
            let t = s.trim();
            if t.is_empty() {
                None
            } else {
                Some(t.to_string())
            }
        }
        Err(_) => None,
    }
}

pub async fn write_update_build_dir(path: Option<&str>) -> Result<(), std::io::Error> {
    if let Some(parent) = Path::new(UPDATE_BUILD_DIR_PATH).parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    match path {
        Some(p) if !p.trim().is_empty() => {
            tokio::fs::write(UPDATE_BUILD_DIR_PATH, format!("{}\n", p.trim())).await
        }
        _ => {
            let _ = tokio::fs::remove_file(UPDATE_BUILD_DIR_PATH).await;
            Ok(())
        }
    }
}

/// Kept for API compatibility; Debian builds do not use a Nix spillover dir.
pub async fn list_bcachefs_pool_mounts() -> Vec<String> {
    Vec::new()
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct UpdateBuildDirConfig {
    pub path: Option<String>,
    pub available_pools: Vec<String>,
    pub resolved: Option<String>,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct VersionInputInfo {
    pub name: String,
    pub url: String,
    pub rev: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tag: Option<String>,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct VersionInfo {
    pub inputs: Vec<VersionInputInfo>,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct VersionTaggedReleaseStatus {
    pub current_url: String,
    pub latest_tag: String,
    pub latest_url: String,
    pub current_is_latest_standard_url: bool,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct BootstrapSystemFlakeResult {
    pub flake_path: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct VersionSwitchInput {
    pub name: String,
    pub url: String,
    #[serde(default)]
    pub update: bool,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct VersionSwitchRequest {
    pub inputs: Vec<VersionSwitchInput>,
}

#[derive(Debug, Error)]
pub enum UpdateError {
    #[error("update already in progress")]
    AlreadyRunning,
    #[error("command failed: {0}")]
    CommandFailed(String),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct UpdateInfo {
    pub current_version: String,
    pub latest_version: Option<String>,
    pub update_available: Option<bool>,
    pub channel: ReleaseChannel,
    pub last_attempt: Option<String>,
    pub error: Option<String>,
    pub inputs: Option<Vec<VersionInputInfo>>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct UpdateStatus {
    pub state: String,
    pub log: String,
    pub reboot_required: bool,
    pub webui_changed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Generation {
    /// Snapper snapshot number (was NixOS generation).
    pub generation: u64,
    pub date: String,
    /// Debian / snapshot description (field name kept for WebUI compat).
    pub nixos_version: String,
    pub kernel_version: String,
    pub nasty_version: Option<String>,
    pub current: bool,
    pub booted: bool,
    pub label: Option<String>,
}

pub struct UpdateService;

impl Default for UpdateService {
    fn default() -> Self {
        Self::new()
    }
}

impl UpdateService {
    pub fn new() -> Self {
        Self
    }

    pub async fn version(&self) -> UpdateInfo {
        UpdateInfo {
            current_version: read_current_version().await,
            latest_version: None,
            update_available: None,
            channel: read_channel().await,
            last_attempt: last_upgrade_attempt_result().await,
            error: None,
            inputs: self.version_info().await.ok().map(|v| v.inputs),
        }
    }

    pub async fn version_info(&self) -> Result<VersionInfo, UpdateError> {
        let nasty = package_version(NASTY_METAPACKAGE)
            .await
            .or(package_version(NASTY_ENGINE_PKG).await)
            .unwrap_or_else(|| "unknown".into());
        let debian = read_debian_version()
            .await
            .unwrap_or_else(|| "unknown".into());
        let kernel = kernel_release();
        Ok(VersionInfo {
            inputs: vec![
                VersionInputInfo {
                    name: "nasty".into(),
                    url: format!("apt:{NASTY_METAPACKAGE}"),
                    rev: Some(nasty.clone()),
                    tag: Some(nasty),
                },
                VersionInputInfo {
                    name: "debian".into(),
                    url: "apt:debian".into(),
                    rev: Some(debian.clone()),
                    tag: Some(debian),
                },
                VersionInputInfo {
                    name: "kernel".into(),
                    url: "uname -r".into(),
                    rev: Some(kernel.clone()),
                    tag: Some(kernel),
                },
            ],
        })
    }

    pub async fn version_tagged_release_status(
        &self,
    ) -> Result<VersionTaggedReleaseStatus, UpdateError> {
        let current = read_current_version().await;
        let token = read_github_token().await;
        let latest = check_latest_channel_release(
            ReleaseChannel::Mild,
            token.as_deref(),
            DEFAULT_NASTY_OWNER,
            DEFAULT_NASTY_REPO,
        )
        .await
        .unwrap_or_else(|_| current.clone());
        let latest_url = format!("github:{DEFAULT_NASTY_OWNER}/{DEFAULT_NASTY_REPO}/{latest}");
        let current_url = format!("apt:{NASTY_METAPACKAGE}={current}");
        Ok(VersionTaggedReleaseStatus {
            current_is_latest_standard_url: current == latest
                || current.trim_start_matches('v') == latest.trim_start_matches('v'),
            current_url,
            latest_tag: latest,
            latest_url,
        })
    }

    pub async fn get_channel(&self) -> ReleaseChannel {
        read_channel().await
    }

    pub async fn set_channel(
        &self,
        channel: ReleaseChannel,
    ) -> Result<ReleaseChannel, UpdateError> {
        if let Some(parent) = Path::new(RELEASE_CHANNEL_PATH).parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        tokio::fs::write(RELEASE_CHANNEL_PATH, format!("{}\n", channel.as_str())).await?;
        info!("Release channel set to {}", channel.display_name());
        Ok(channel)
    }

    pub async fn get_update_build_dir(&self) -> UpdateBuildDirConfig {
        UpdateBuildDirConfig {
            path: read_update_build_dir().await,
            available_pools: Vec::new(),
            resolved: None,
        }
    }

    pub async fn set_update_build_dir(
        &self,
        path: Option<String>,
    ) -> Result<UpdateBuildDirConfig, UpdateError> {
        write_update_build_dir(path.as_deref()).await?;
        Ok(self.get_update_build_dir().await)
    }

    /// Debian: tagged-release upgrade is the same apt path as apply().
    pub async fn upgrade_tagged_release(&self) -> Result<(), UpdateError> {
        self.apply().await
    }

    pub async fn reboot_required(&self) -> bool {
        is_reboot_required_pub().await
    }

    pub async fn reboot(&self) -> Result<(), UpdateError> {
        info!("System reboot requested");
        let output = tokio::process::Command::new("systemctl")
            .arg("reboot")
            .output()
            .await
            .map_err(|e| UpdateError::CommandFailed(format!("systemctl reboot: {e}")))?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(UpdateError::CommandFailed(format!(
                "reboot failed: {stderr}"
            )));
        }
        Ok(())
    }

    pub async fn shutdown(&self) -> Result<(), UpdateError> {
        info!("System shutdown requested");
        let output = tokio::process::Command::new("systemctl")
            .arg("poweroff")
            .output()
            .await
            .map_err(|e| UpdateError::CommandFailed(format!("systemctl poweroff: {e}")))?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(UpdateError::CommandFailed(format!(
                "shutdown failed: {stderr}"
            )));
        }
        Ok(())
    }

    pub async fn check(&self) -> Result<UpdateInfo, UpdateError> {
        let current = read_current_version().await;
        let channel = read_channel().await;
        let mut lookup_error: Option<String> = None;

        // Prefer apt candidate when a nasty package is installed from a repo.
        let apt_candidate = apt_candidate_version(NASTY_METAPACKAGE)
            .await
            .or(apt_candidate_version(NASTY_ENGINE_PKG).await);

        let latest = match apt_candidate {
            Some(ref cand) if channel == ReleaseChannel::Nasty || cand != &current => cand.clone(),
            _ => {
                let token = read_github_token().await;
                match check_latest_channel_release(
                    channel,
                    token.as_deref(),
                    DEFAULT_NASTY_OWNER,
                    DEFAULT_NASTY_REPO,
                )
                .await
                {
                    Ok(tag) => tag,
                    Err(e) => {
                        warn!(target: "nasty::update", "release check failed: {e}");
                        lookup_error = Some(e.to_string());
                        if let Some(cand) = apt_candidate {
                            cand
                        } else {
                            "unknown".into()
                        }
                    }
                }
            }
        };

        let current_clean = current.trim_end_matches("-dirty");
        let mut available = if latest == "unknown" {
            None
        } else if current_clean == "dev" {
            Some(true)
        } else {
            Some(normalize_ver(current_clean) != normalize_ver(&latest))
        };

        if matches!(
            last_upgrade_attempt_result().await.as_deref(),
            Some("failed")
        ) {
            available = Some(true);
        }

        // Also flag when any of our packages have a newer apt candidate.
        if available != Some(true) {
            for pkg in [NASTY_METAPACKAGE, NASTY_ENGINE_PKG, NASTY_WEBUI_PKG] {
                if apt_upgrade_pending(pkg).await {
                    available = Some(true);
                    break;
                }
            }
        }

        Ok(UpdateInfo {
            current_version: current,
            latest_version: if latest == "unknown" {
                None
            } else {
                Some(latest)
            },
            update_available: available,
            channel,
            last_attempt: last_upgrade_attempt_result().await,
            error: lookup_error,
            inputs: self.version_info().await.ok().map(|v| v.inputs),
        })
    }

    pub async fn apply(&self) -> Result<(), UpdateError> {
        ensure_update_idle().await?;
        let script = r#"#!/bin/bash
set -euo pipefail
LOG=/var/lib/nasty/update.log
mkdir -p /var/lib/nasty
exec > >(tee -a "$LOG") 2>&1
echo "==> Starting apt upgrade ($(date -Is))"
echo "failed" > /var/lib/nasty/last-upgrade-attempt
WEBUI_BEFORE=$(dpkg-query -W -f='${Version}' nasty-webui 2>/dev/null || true)

if command -v snapper >/dev/null 2>&1; then
  if snapper -c root get-config >/dev/null 2>&1; then
    echo "==> Creating snapper pre-snapshot"
    snapper -c root create --type pre --cleanup-algorithm number \
      --description "nasty apt upgrade" --userdata "nasty=upgrade" || true
  else
    echo "==> snapper config 'root' missing; skipping explicit pre-snapshot"
    echo "    (install snapper and run: snapper -c root create-config /)"
  fi
else
  echo "==> snapper not installed; continuing without snapshots"
fi

export DEBIAN_FRONTEND=noninteractive
echo "==> apt-get update"
apt-get update -y
echo "==> apt-get upgrade"
apt-get -y -o Dpkg::Options::=--force-confdef -o Dpkg::Options::=--force-confold \
  upgrade --with-new-pkgs
# Prefer upgrading our packages even if held back from a full upgrade path.
apt-get -y -o Dpkg::Options::=--force-confdef -o Dpkg::Options::=--force-confold \
  install --only-upgrade nasty nasty-engine nasty-webui || true

WEBUI_AFTER=$(dpkg-query -W -f='${Version}' nasty-webui 2>/dev/null || true)
if [ -n "$WEBUI_BEFORE" ] && [ -n "$WEBUI_AFTER" ] && [ "$WEBUI_BEFORE" != "$WEBUI_AFTER" ]; then
  touch /var/lib/nasty/update-webui-changed
fi

# Persist package version for the Version page.
VER=$(dpkg-query -W -f='${Version}' nasty 2>/dev/null \
  || dpkg-query -W -f='${Version}' nasty-engine 2>/dev/null \
  || true)
if [ -n "${VER:-}" ]; then
  printf '%s\n' "$VER" > /var/lib/nasty/version
  printf '%s\n' "$VER" > /etc/nasty-version
fi

echo "success" > /var/lib/nasty/last-upgrade-attempt
echo "==> Upgrade finished ($(date -Is))"
"#;
        start_transient_update_unit(script).await
    }

    pub async fn version_switch(&self, _req: VersionSwitchRequest) -> Result<(), UpdateError> {
        Err(UpdateError::CommandFailed(
            "flake input switching is not available on Debian; install/upgrade .deb packages via apt"
                .into(),
        ))
    }

    pub async fn rollback(&self) -> Result<(), UpdateError> {
        ensure_update_idle().await?;
        // Roll back to the previous snapper snapshot (highest number below current default).
        let gens = self.list_generations().await?;
        let target = gens
            .iter()
            .filter(|g| !g.current && !g.booted)
            .max_by_key(|g| g.generation)
            .or_else(|| {
                gens.iter()
                    .filter(|g| !g.current)
                    .max_by_key(|g| g.generation)
            });
        let Some(target) = target else {
            return Err(UpdateError::CommandFailed(
                "no snapper snapshot available to roll back to".into(),
            ));
        };
        self.switch_generation(target.generation).await
    }

    pub async fn list_generations(&self) -> Result<Vec<Generation>, UpdateError> {
        let labels = read_generation_labels().await;
        let kernel = kernel_release();
        let nasty = package_version(NASTY_METAPACKAGE)
            .await
            .or(package_version(NASTY_ENGINE_PKG).await);
        let debian = read_debian_version()
            .await
            .unwrap_or_else(|| "Debian".into());

        let output = tokio::process::Command::new("snapper")
            .args(["--jsonout", "-c", SNAPPER_CONFIG, "list"])
            .output()
            .await
            .map_err(|e| UpdateError::CommandFailed(format!("snapper list: {e}")))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(UpdateError::CommandFailed(format!(
                "snapper list failed: {stderr}"
            )));
        }

        let parsed: SnapperListJson = serde_json::from_slice(&output.stdout).map_err(|e| {
            UpdateError::CommandFailed(format!("snapper --jsonout parse error: {e}"))
        })?;

        let mut gens = Vec::new();
        for snap in parsed.snapshots.into_iter().chain(parsed.root.into_iter()) {
            if snap.number == 0 {
                // snapper "current" synthetic entry
                continue;
            }
            let label = labels.get(&snap.number).cloned().or_else(|| {
                let d = snap.description.trim();
                if d.is_empty() {
                    None
                } else {
                    Some(d.to_string())
                }
            });
            let desc = if snap.description.trim().is_empty() {
                format!("{debian} ({})", snap.snapshot_type)
            } else {
                snap.description.clone()
            };
            gens.push(Generation {
                generation: snap.number,
                date: snap.date_string(),
                nixos_version: desc,
                kernel_version: kernel.clone(),
                nasty_version: nasty.clone(),
                current: snap.default || snap.active,
                booted: snap.active,
                label,
            });
        }
        gens.sort_by_key(|g| g.generation);
        Ok(gens)
    }

    pub async fn switch_generation(&self, gen_id: u64) -> Result<(), UpdateError> {
        ensure_update_idle().await?;
        let script = format!(
            r#"#!/bin/bash
set -euo pipefail
LOG=/var/lib/nasty/update.log
mkdir -p /var/lib/nasty
exec > >(tee -a "$LOG") 2>&1
echo "==> Switching to generation {gen_id}"
echo "failed" > /var/lib/nasty/last-upgrade-attempt
echo "==> Activating generation {gen_id} via snapper rollback"
snapper -c root --ambit classic rollback {gen_id}
echo "success" > /var/lib/nasty/last-upgrade-attempt
echo "==> Switch to generation {gen_id} complete — reboot required"
"#
        );
        start_transient_update_unit(&script).await
    }

    pub async fn label_generation(
        &self,
        gen_id: u64,
        label: Option<String>,
    ) -> Result<(), UpdateError> {
        let mut labels = read_generation_labels().await;
        match label {
            Some(l) if !l.trim().is_empty() => {
                labels.insert(gen_id, l.trim().to_string());
            }
            _ => {
                labels.remove(&gen_id);
            }
        }
        write_generation_labels(&labels).await?;
        // Also push into snapper description when possible.
        if let Some(l) = labels.get(&gen_id) {
            let _ = tokio::process::Command::new("snapper")
                .args(["-c", SNAPPER_CONFIG, "modify", "--description", l])
                .arg(gen_id.to_string())
                .status()
                .await;
        }
        Ok(())
    }

    pub async fn delete_generation(&self, gen_id: u64) -> Result<(), UpdateError> {
        let status = tokio::process::Command::new("snapper")
            .args(["-c", SNAPPER_CONFIG, "delete", &gen_id.to_string()])
            .status()
            .await
            .map_err(|e| UpdateError::CommandFailed(format!("snapper delete: {e}")))?;
        if !status.success() {
            return Err(UpdateError::CommandFailed(format!(
                "snapper delete {gen_id} failed"
            )));
        }
        let mut labels = read_generation_labels().await;
        labels.remove(&gen_id);
        write_generation_labels(&labels).await?;
        Ok(())
    }

    pub async fn status(&self) -> UpdateStatus {
        let state = update_unit_state().await;
        let log = tokio::fs::read_to_string("/var/lib/nasty/update.log")
            .await
            .unwrap_or_default();
        let webui_changed = Path::new(UPDATE_WEBUI_CHANGED).exists();
        let reboot_required = is_reboot_required_pub().await;
        UpdateStatus {
            state,
            log,
            reboot_required,
            webui_changed,
        }
    }
}

// ── Snapper JSON ────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct SnapperListJson {
    #[serde(default)]
    snapshots: Vec<SnapperSnapshot>,
    /// Some snapper versions nest under a config key.
    #[serde(default)]
    root: Vec<SnapperSnapshot>,
}

#[derive(Debug, Deserialize)]
struct SnapperSnapshot {
    number: u64,
    #[serde(default, rename = "type")]
    snapshot_type: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    date: Option<serde_json::Value>,
    #[serde(default)]
    default: bool,
    #[serde(default)]
    active: bool,
    #[serde(default)]
    userdata: HashMap<String, String>,
}

impl SnapperSnapshot {
    fn date_string(&self) -> String {
        match &self.date {
            Some(serde_json::Value::String(s)) => s.clone(),
            Some(serde_json::Value::Number(n)) => n.to_string(),
            _ => self.userdata.get("date").cloned().unwrap_or_default(),
        }
    }
}

// ── Helpers ─────────────────────────────────────────────────────

async fn read_current_version() -> String {
    for path in [VERSION_PATH, VERSION_PATH_FALLBACK] {
        if let Ok(s) = tokio::fs::read_to_string(path).await {
            let t = s.trim();
            if !t.is_empty() {
                return t.to_string();
            }
        }
    }
    if let Some(v) = package_version(NASTY_METAPACKAGE).await {
        return v;
    }
    if let Some(v) = package_version(NASTY_ENGINE_PKG).await {
        return v;
    }
    option_env!("NASTY_GIT_SHA")
        .map(|s| {
            if s.len() > 12 {
                s[..12].to_string()
            } else {
                s.to_string()
            }
        })
        .unwrap_or_else(|| env!("CARGO_PKG_VERSION").to_string())
}

async fn package_version(pkg: &str) -> Option<String> {
    let output = tokio::process::Command::new("dpkg-query")
        .args(["-W", "-f=${Version}", pkg])
        .output()
        .await
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let v = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if v.is_empty() { None } else { Some(v) }
}

async fn apt_candidate_version(pkg: &str) -> Option<String> {
    let output = tokio::process::Command::new("apt-cache")
        .args(["policy", pkg])
        .output()
        .await
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout);
    for line in text.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("Candidate:") {
            let cand = rest.trim();
            if cand.is_empty() || cand == "(none)" {
                return None;
            }
            return Some(cand.to_string());
        }
    }
    None
}

async fn apt_upgrade_pending(pkg: &str) -> bool {
    let Some(installed) = package_version(pkg).await else {
        return false;
    };
    let Some(candidate) = apt_candidate_version(pkg).await else {
        return false;
    };
    normalize_ver(&installed) != normalize_ver(&candidate)
}

fn normalize_ver(v: &str) -> String {
    v.trim()
        .trim_start_matches('v')
        .split_once('-')
        .map(|(a, _)| a)
        .unwrap_or(v.trim().trim_start_matches('v'))
        .to_string()
}

async fn read_debian_version() -> Option<String> {
    tokio::fs::read_to_string("/etc/debian_version")
        .await
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

fn kernel_release() -> String {
    std::fs::read_to_string("/proc/sys/kernel/osrelease")
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|_| "unknown".into())
}

async fn last_upgrade_attempt_result() -> Option<String> {
    tokio::fs::read_to_string(LAST_ATTEMPT_PATH)
        .await
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

async fn read_generation_labels() -> HashMap<u64, String> {
    match tokio::fs::read_to_string(GENERATION_LABELS_PATH).await {
        Ok(s) => serde_json::from_str(&s).unwrap_or_default(),
        Err(_) => HashMap::new(),
    }
}

async fn write_generation_labels(labels: &HashMap<u64, String>) -> Result<(), UpdateError> {
    if let Some(parent) = Path::new(GENERATION_LABELS_PATH).parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    let json = serde_json::to_string_pretty(labels)
        .map_err(|e| UpdateError::CommandFailed(e.to_string()))?;
    tokio::fs::write(GENERATION_LABELS_PATH, json).await?;
    Ok(())
}

async fn ensure_update_idle() -> Result<(), UpdateError> {
    let state = update_unit_state().await;
    if state == "running" {
        return Err(UpdateError::AlreadyRunning);
    }
    Ok(())
}

async fn update_unit_state() -> String {
    let output = tokio::process::Command::new("systemctl")
        .args(["is-active", &format!("{UPDATE_UNIT}.service")])
        .output()
        .await;
    match output {
        Ok(o) => {
            let s = String::from_utf8_lossy(&o.stdout).trim().to_string();
            match s.as_str() {
                "active" | "activating" => "running".into(),
                _ => {
                    // Check result of last run.
                    match last_upgrade_attempt_result().await.as_deref() {
                        Some("failed") => "failed".into(),
                        Some("success") => "success".into(),
                        _ => "idle".into(),
                    }
                }
            }
        }
        Err(_) => "idle".into(),
    }
}

async fn start_transient_update_unit(script: &str) -> Result<(), UpdateError> {
    let script_path = "/run/nasty-update.sh";
    tokio::fs::write(script_path, script).await?;
    // systemd-run needs the file executable.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(script_path)?.permissions();
        perms.set_mode(0o700);
        std::fs::set_permissions(script_path, perms)?;
    }

    // Clear previous log so status polling shows this run.
    let _ = tokio::fs::remove_file("/var/lib/nasty/update.log").await;
    let _ = tokio::fs::remove_file(UPDATE_WEBUI_CHANGED).await;

    let status = tokio::process::Command::new("systemd-run")
        .args([
            "--unit",
            UPDATE_UNIT,
            "--collect",
            "--property=Type=oneshot",
            "--property=RemainAfterExit=yes",
            "/bin/bash",
            script_path,
        ])
        .status()
        .await
        .map_err(|e| UpdateError::CommandFailed(format!("systemd-run: {e}")))?;

    if !status.success() {
        return Err(UpdateError::CommandFailed(
            "failed to start nasty-update transient unit".into(),
        ));
    }
    info!(target: "nasty::update", "started {UPDATE_UNIT}.service");
    Ok(())
}

async fn read_github_token() -> Option<String> {
    for path in [
        "/var/lib/nasty/github-token",
        "/etc/nasty/github-token",
        "/root/.github-token",
    ] {
        if let Ok(s) = tokio::fs::read_to_string(path).await {
            let t = s.trim();
            if !t.is_empty() {
                return Some(t.to_string());
            }
        }
    }
    std::env::var("GITHUB_TOKEN").ok().filter(|s| !s.is_empty())
}

async fn check_latest_channel_release(
    channel: ReleaseChannel,
    token: Option<&str>,
    owner: &str,
    repo: &str,
) -> Result<String, UpdateError> {
    let url = format!("https://api.github.com/repos/{owner}/{repo}/releases?per_page=30");
    let mut req = reqwest::Client::builder()
        .timeout(GITHUB_FETCH_TIMEOUT)
        .build()
        .map_err(|e| UpdateError::CommandFailed(e.to_string()))?
        .get(&url)
        .header("User-Agent", "nasty-engine");
    if let Some(t) = token {
        req = req.bearer_auth(t);
    }
    let releases: Vec<GitHubRelease> = req
        .send()
        .await
        .map_err(|e| UpdateError::CommandFailed(format!("GitHub releases: {e}")))?
        .error_for_status()
        .map_err(|e| UpdateError::CommandFailed(format!("GitHub releases: {e}")))?
        .json()
        .await
        .map_err(|e| UpdateError::CommandFailed(format!("GitHub releases JSON: {e}")))?;

    let tag = releases
        .into_iter()
        .filter(|r| !r.draft)
        .map(|r| r.tag_name)
        .find(|t| match channel {
            ReleaseChannel::Mild => t.starts_with('v') && !t.contains("rc") && !t.contains("alpha"),
            ReleaseChannel::Spicy => t.starts_with('s') || t.starts_with('v'),
            ReleaseChannel::Nasty => true,
        })
        .ok_or_else(|| UpdateError::CommandFailed("no matching GitHub release".into()))?;
    Ok(tag)
}

/// Reboot needed when `/var/run/reboot-required` exists (Debian) or
/// snapper default subvolume differs from the booted one.
pub async fn is_reboot_required_pub() -> bool {
    if Path::new("/run/reboot-required").exists() || Path::new("/var/run/reboot-required").exists()
    {
        return true;
    }
    // Heuristic: last update log asked for reboot.
    if let Ok(log) = tokio::fs::read_to_string("/var/lib/nasty/update.log").await {
        if log.contains("reboot required") || log.contains("Reboot required") {
            return true;
        }
    }
    false
}

/// Debian port: no flake.lock pin.
pub async fn read_flake_lock_bcachefs_pub() -> (Option<String>, Option<String>) {
    (None, None)
}

/// Debian port: no embedded flake defaults.
pub fn embedded_default_bcachefs_tools_ref() -> Result<String, UpdateError> {
    Err(UpdateError::CommandFailed(
        "bcachefs flake pin is not available on Debian builds".into(),
    ))
}

pub async fn bootstrap_system_flake_from_template_path(
    _template_path: &str,
    _dest_dir: &str,
    _nasty_version: &str,
    _local_system: &str,
) -> Result<BootstrapSystemFlakeResult, UpdateError> {
    Err(UpdateError::CommandFailed(
        "NixOS system-flake bootstrap is not available on Debian; use the nasty .deb packages"
            .into(),
    ))
}

pub async fn bootstrap_system_flake_from_template(
    _template: &str,
    _dest_dir: &str,
    _nasty_version: &str,
    _local_system: &str,
) -> Result<BootstrapSystemFlakeResult, UpdateError> {
    bootstrap_system_flake_from_template_path("", "", "", "").await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_strips_v_and_revision() {
        assert_eq!(normalize_ver("v0.0.15"), "0.0.15");
        assert_eq!(normalize_ver("0.0.15-1"), "0.0.15");
    }

    #[test]
    fn channel_roundtrip() {
        assert_eq!(ReleaseChannel::parse("mild"), Some(ReleaseChannel::Mild));
        assert_eq!(ReleaseChannel::Nasty.as_str(), "nasty");
    }
}
