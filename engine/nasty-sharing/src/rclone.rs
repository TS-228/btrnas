//! FTP, SFTP, and S3 file serving via `rclone serve`.
//!
//! Each protocol is one systemd unit (`nasty-rclone@{ftp,sftp,s3}.service`) that
//! presents every enabled share as a top-level folder (FTP/SFTP) or bucket
//! (S3) through rclone's `combine` backend. Credentials and listen ports are
//! per-protocol, not per-share — rclone serve authenticates a single user.

use std::os::unix::fs::PermissionsExt;
use std::path::Path;

use nasty_common::secrets::{self, EncryptedBlob, SecretError};
use nasty_common::{HasId, StateDir};
use rand::distr::{Alphanumeric, SampleString};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tracing::{info, warn};
use uuid::Uuid;

const RCLONE_DIR: &str = "/var/lib/nasty/rclone";
const EMPTY_DIR: &str = "/var/lib/nasty/rclone/empty";
const SFTP_HOST_KEY: &str = "/var/lib/nasty/rclone/sftp_host_key";
const PASSWORD_LEN: usize = 24;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RcloneKind {
    Ftp,
    Sftp,
    S3,
}

impl RcloneKind {
    pub fn name(self) -> &'static str {
        match self {
            Self::Ftp => "ftp",
            Self::Sftp => "sftp",
            Self::S3 => "s3",
        }
    }

    pub fn display_name(self) -> &'static str {
        match self {
            Self::Ftp => "FTP",
            Self::Sftp => "SFTP",
            Self::S3 => "S3",
        }
    }

    pub fn default_port(self) -> u16 {
        match self {
            Self::Ftp => 21,
            Self::Sftp => 2022,
            Self::S3 => 9000,
        }
    }

    pub fn unit(self) -> &'static str {
        match self {
            Self::Ftp => "nasty-rclone@ftp.service",
            Self::Sftp => "nasty-rclone@sftp.service",
            Self::S3 => "nasty-rclone@s3.service",
        }
    }

    fn state_dir(self) -> String {
        format!("/var/lib/nasty/shares/{}", self.name())
    }

    fn conf_path(self) -> String {
        format!("{RCLONE_DIR}/{}.conf", self.name())
    }

    fn script_path(self) -> String {
        format!("{RCLONE_DIR}/{}-serve.sh", self.name())
    }

    fn settings_path(self) -> String {
        format!("{RCLONE_DIR}/{}-server.json", self.name())
    }

    fn secrets_name(self) -> &'static str {
        match self {
            Self::Ftp => "nasty.rclone.ftp.password",
            Self::Sftp => "nasty.rclone.sftp.password",
            Self::S3 => "nasty.rclone.s3.password",
        }
    }

    const DEFAULT_USERNAME: &'static str = "nasty";
    const DEFAULT_PASSIVE_MIN: u16 = 30000;
    const DEFAULT_PASSIVE_MAX: u16 = 30100;
}

#[derive(Debug, Error)]
pub enum RcloneError {
    #[error("share not found: {0}")]
    NotFound(String),
    #[error("share name already exists: {0}")]
    NameExists(String),
    #[error("path does not exist: {0}")]
    PathNotFound(String),
    #[error("path is not within a NASty filesystem: {0}")]
    PathNotInFilesystem(String),
    #[error("invalid share name: {0}")]
    InvalidName(String),
    #[error("invalid share path: {0}")]
    InvalidPath(String),
    #[error("invalid settings: {0}")]
    InvalidSettings(String),
    #[error("encrypt credential failed: {0}")]
    Encrypt(SecretError),
    #[error("decrypt credential failed: {0}")]
    Decrypt(SecretError),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct RcloneShare {
    /// Unique share identifier (UUID).
    pub id: String,
    /// Folder (FTP/SFTP) or bucket (S3) name presented to clients.
    pub name: String,
    /// Absolute filesystem path being shared (must be under `/fs/`).
    pub path: String,
    /// Optional description.
    pub comment: Option<String>,
    /// Whether the share is read-only.
    pub read_only: bool,
    /// Whether the share is currently served.
    pub enabled: bool,
}

impl HasId for RcloneShare {
    fn id(&self) -> &str {
        &self.id
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct CreateRcloneShareRequest {
    /// Folder / bucket name (1–80 characters: letters, digits, `.`, `_`, `-`).
    pub name: String,
    /// Absolute path to share (must exist and be under `/fs/`).
    pub path: String,
    /// Optional description.
    pub comment: Option<String>,
    /// Whether the share is read-only (default: false).
    pub read_only: Option<bool>,
    /// Whether to enable the share immediately (default: true).
    pub enabled: Option<bool>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct UpdateRcloneShareRequest {
    /// ID of the share to update.
    pub id: String,
    /// New folder / bucket name (optional; must be unique).
    pub name: Option<String>,
    /// New description (optional).
    pub comment: Option<String>,
    /// Update read-only flag (optional).
    pub read_only: Option<bool>,
    /// Enable or disable the share (optional).
    pub enabled: Option<bool>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct DeleteRcloneShareRequest {
    pub id: String,
}

/// Live server settings returned to the WebUI, including the decrypted secret.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct RcloneSettings {
    /// Bind address (default `0.0.0.0`).
    pub listen: String,
    /// TCP listen port.
    pub port: u16,
    /// FTP/SFTP username, or S3 access key.
    pub username: String,
    /// FTP/SFTP password, or S3 secret key.
    pub password: String,
    /// FTP only: accept any password as `anonymous` (default false).
    pub anonymous: bool,
    /// FTP only: start of PASV port range.
    pub passive_port_min: u16,
    /// FTP only: end of PASV port range.
    pub passive_port_max: u16,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct UpdateRcloneSettingsRequest {
    pub listen: Option<String>,
    pub port: Option<u16>,
    pub username: Option<String>,
    pub password: Option<String>,
    pub anonymous: Option<bool>,
    pub passive_port_min: Option<u16>,
    pub passive_port_max: Option<u16>,
}

/// Ports the firewall should open: control port, plus FTP PASV range.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RcloneListenPorts {
    pub port: u16,
    pub passive: Option<(u16, u16)>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct StoredServer {
    #[serde(default)]
    listen: String,
    #[serde(default)]
    port: u16,
    #[serde(default)]
    username: String,
    password_encrypted: EncryptedBlob,
    #[serde(default)]
    anonymous: bool,
    #[serde(default)]
    passive_port_min: u16,
    #[serde(default)]
    passive_port_max: u16,
}

pub struct RcloneService {
    kind: RcloneKind,
}

impl RcloneService {
    pub fn ftp() -> Self {
        Self {
            kind: RcloneKind::Ftp,
        }
    }

    pub fn sftp() -> Self {
        Self {
            kind: RcloneKind::Sftp,
        }
    }

    pub fn s3() -> Self {
        Self {
            kind: RcloneKind::S3,
        }
    }

    pub fn kind(&self) -> RcloneKind {
        self.kind
    }

    fn state(&self) -> StateDir {
        StateDir::new(self.kind.state_dir())
    }

    pub async fn list(&self) -> Result<Vec<RcloneShare>, RcloneError> {
        Ok(self.state().load_all().await)
    }

    pub async fn list_strict(&self) -> Result<Vec<RcloneShare>, RcloneError> {
        Ok(self.state().load_all_strict().await?)
    }

    pub async fn get(&self, id: &str) -> Result<RcloneShare, RcloneError> {
        self.state()
            .load::<RcloneShare>(id)
            .await
            .ok_or_else(|| RcloneError::NotFound(id.to_string()))
    }

    pub async fn create(&self, req: CreateRcloneShareRequest) -> Result<RcloneShare, RcloneError> {
        validate_share_name(&req.name)?;
        let path = validate_and_canonicalize_path(&req.path)?;

        let shares: Vec<RcloneShare> = self.state().load_all().await;
        if let Some(existing) = shares.into_iter().find(|s| s.name == req.name) {
            info!(
                "{} share '{}' already exists, returning existing (idempotent)",
                self.kind.display_name(),
                req.name
            );
            return Ok(existing);
        }

        let share = RcloneShare {
            id: Uuid::new_v4().to_string(),
            name: req.name,
            path,
            comment: req.comment,
            read_only: req.read_only.unwrap_or(false),
            enabled: req.enabled.unwrap_or(true),
        };
        self.state().save(&share.id, &share).await?;
        self.apply().await?;
        info!(
            "Created {} share '{}' at {}",
            self.kind.display_name(),
            share.name,
            share.path
        );
        Ok(share)
    }

    pub async fn update(&self, req: UpdateRcloneShareRequest) -> Result<RcloneShare, RcloneError> {
        let mut share: RcloneShare = self
            .state()
            .load(&req.id)
            .await
            .ok_or_else(|| RcloneError::NotFound(req.id.clone()))?;

        if let Some(name) = req.name {
            validate_share_name(&name)?;
            if name != share.name {
                let shares: Vec<RcloneShare> = self.state().load_all().await;
                if shares.iter().any(|s| s.id != share.id && s.name == name) {
                    return Err(RcloneError::NameExists(name));
                }
            }
            share.name = name;
        }
        if let Some(comment) = req.comment {
            share.comment = Some(comment);
        }
        if let Some(read_only) = req.read_only {
            share.read_only = read_only;
        }
        if let Some(enabled) = req.enabled {
            share.enabled = enabled;
        }

        self.state().save(&share.id, &share).await?;
        self.apply().await?;
        info!("Updated {} share '{}'", self.kind.display_name(), share.id);
        Ok(share)
    }

    pub async fn delete(&self, req: DeleteRcloneShareRequest) -> Result<(), RcloneError> {
        let share: RcloneShare = self
            .state()
            .load(&req.id)
            .await
            .ok_or_else(|| RcloneError::NotFound(req.id.clone()))?;
        self.state().remove(&req.id).await?;
        self.apply().await?;
        info!(
            "Deleted {} share '{}'",
            self.kind.display_name(),
            share.name
        );
        Ok(())
    }

    /// Write rclone.conf + serve script (and credentials if missing). Safe on
    /// every boot before the systemd unit starts.
    pub async fn ensure_config(&self) -> Result<(), RcloneError> {
        self.ensure_settings().await?;
        if self.kind == RcloneKind::Sftp {
            ensure_sftp_host_key().await;
        }
        self.apply().await
    }

    pub async fn settings(&self) -> Result<RcloneSettings, RcloneError> {
        let stored = self.ensure_settings().await?;
        decrypt_settings(self.kind, &stored).await
    }

    pub async fn update_settings(
        &self,
        req: UpdateRcloneSettingsRequest,
    ) -> Result<RcloneSettings, RcloneError> {
        let mut stored = self.ensure_settings().await?;
        if let Some(listen) = req.listen {
            validate_listen(&listen)?;
            stored.listen = listen;
        }
        if let Some(port) = req.port {
            validate_port(self.kind, port)?;
            stored.port = port;
        }
        if let Some(username) = req.username {
            validate_username(&username)?;
            stored.username = username;
        }
        if let Some(password) = req.password {
            validate_password(&password)?;
            stored.password_encrypted = secrets::encrypt(self.kind.secrets_name(), &password)
                .await
                .map_err(RcloneError::Encrypt)?;
        }
        if let Some(anonymous) = req.anonymous {
            stored.anonymous = anonymous && self.kind == RcloneKind::Ftp;
        }
        if let Some(min) = req.passive_port_min {
            stored.passive_port_min = min;
        }
        if let Some(max) = req.passive_port_max {
            stored.passive_port_max = max;
        }
        validate_passive_range(stored.passive_port_min, stored.passive_port_max)?;
        save_stored(self.kind, &stored).await?;
        self.apply().await?;
        decrypt_settings(self.kind, &stored).await
    }

    pub async fn listen_ports(&self) -> RcloneListenPorts {
        let stored = load_stored(self.kind).await.ok().flatten();
        let port = stored
            .as_ref()
            .map(|s| s.port)
            .filter(|p| *p > 0)
            .unwrap_or_else(|| self.kind.default_port());
        let passive = if self.kind == RcloneKind::Ftp {
            let min = stored
                .as_ref()
                .map(|s| s.passive_port_min)
                .filter(|p| *p > 0)
                .unwrap_or(RcloneKind::DEFAULT_PASSIVE_MIN);
            let max = stored
                .as_ref()
                .map(|s| s.passive_port_max)
                .filter(|p| *p > 0)
                .unwrap_or(RcloneKind::DEFAULT_PASSIVE_MAX);
            Some((min, max))
        } else {
            None
        };
        RcloneListenPorts { port, passive }
    }

    async fn ensure_settings(&self) -> Result<StoredServer, RcloneError> {
        if let Some(stored) = load_stored(self.kind).await? {
            return Ok(stored);
        }
        let password = generate_password();
        let stored = StoredServer {
            listen: "0.0.0.0".to_string(),
            port: self.kind.default_port(),
            username: RcloneKind::DEFAULT_USERNAME.to_string(),
            password_encrypted: secrets::encrypt(self.kind.secrets_name(), &password)
                .await
                .map_err(RcloneError::Encrypt)?,
            anonymous: false,
            passive_port_min: RcloneKind::DEFAULT_PASSIVE_MIN,
            passive_port_max: RcloneKind::DEFAULT_PASSIVE_MAX,
        };
        save_stored(self.kind, &stored).await?;
        info!(
            "{}: generated initial credentials (user {})",
            self.kind.display_name(),
            stored.username
        );
        Ok(stored)
    }

    async fn apply(&self) -> Result<(), RcloneError> {
        tokio::fs::create_dir_all(RCLONE_DIR).await?;
        tokio::fs::create_dir_all(EMPTY_DIR).await?;
        let shares = self.list().await?;
        let stored = match load_stored(self.kind).await? {
            Some(s) => s,
            None => return Ok(()),
        };
        let settings = decrypt_settings(self.kind, &stored).await?;
        let conf = render_rclone_conf(&shares);
        let script = render_serve_script(self.kind, &settings, &shares);
        atomic_write(&self.kind.conf_path(), conf.as_bytes(), 0o600).await?;
        atomic_write(&self.kind.script_path(), script.as_bytes(), 0o700).await?;
        nasty_common::cmd::try_run("systemctl", &["try-restart", self.kind.unit()]).await;
        Ok(())
    }
}

async fn decrypt_settings(
    kind: RcloneKind,
    stored: &StoredServer,
) -> Result<RcloneSettings, RcloneError> {
    let password = secrets::decrypt(kind.secrets_name(), &stored.password_encrypted)
        .await
        .map_err(RcloneError::Decrypt)?;
    Ok(RcloneSettings {
        listen: if stored.listen.is_empty() {
            "0.0.0.0".into()
        } else {
            stored.listen.clone()
        },
        port: if stored.port == 0 {
            kind.default_port()
        } else {
            stored.port
        },
        username: if stored.username.is_empty() {
            RcloneKind::DEFAULT_USERNAME.into()
        } else {
            stored.username.clone()
        },
        password,
        anonymous: stored.anonymous && kind == RcloneKind::Ftp,
        passive_port_min: if stored.passive_port_min == 0 {
            RcloneKind::DEFAULT_PASSIVE_MIN
        } else {
            stored.passive_port_min
        },
        passive_port_max: if stored.passive_port_max == 0 {
            RcloneKind::DEFAULT_PASSIVE_MAX
        } else {
            stored.passive_port_max
        },
    })
}

async fn load_stored(kind: RcloneKind) -> Result<Option<StoredServer>, RcloneError> {
    match tokio::fs::read_to_string(kind.settings_path()).await {
        Ok(body) => serde_json::from_str(&body)
            .map(Some)
            .map_err(|e| RcloneError::InvalidSettings(e.to_string())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e.into()),
    }
}

async fn save_stored(kind: RcloneKind, stored: &StoredServer) -> Result<(), RcloneError> {
    tokio::fs::create_dir_all(RCLONE_DIR).await?;
    let json = serde_json::to_string_pretty(stored)
        .map_err(|e| RcloneError::InvalidSettings(e.to_string()))?;
    atomic_write(&kind.settings_path(), json.as_bytes(), 0o600).await
}

async fn atomic_write(path: &str, bytes: &[u8], mode: u32) -> Result<(), RcloneError> {
    let tmp = format!("{path}.tmp");
    tokio::fs::write(&tmp, bytes).await?;
    tokio::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(mode)).await?;
    tokio::fs::rename(&tmp, path).await?;
    Ok(())
}

async fn ensure_sftp_host_key() {
    if Path::new(SFTP_HOST_KEY).exists() {
        return;
    }
    if let Err(e) = tokio::fs::create_dir_all(RCLONE_DIR).await {
        warn!("rclone sftp: create {RCLONE_DIR} failed: {e}");
        return;
    }
    let status = nasty_common::cmd::run(
        "ssh-keygen",
        &["-t", "ed25519", "-f", SFTP_HOST_KEY, "-N", "", "-q"],
    )
    .await;
    match status {
        Ok(out) if out.status.success() => {
            let _ =
                tokio::fs::set_permissions(SFTP_HOST_KEY, std::fs::Permissions::from_mode(0o600))
                    .await;
            info!("rclone sftp: generated host key at {SFTP_HOST_KEY}");
        }
        Ok(out) => warn!(
            "rclone sftp: ssh-keygen failed: {}",
            String::from_utf8_lossy(&out.stderr)
        ),
        Err(e) => warn!("rclone sftp: ssh-keygen spawn failed: {e}"),
    }
}

fn generate_password() -> String {
    Alphanumeric.sample_string(&mut rand::rng(), PASSWORD_LEN)
}

fn validate_share_name(name: &str) -> Result<(), RcloneError> {
    if name.is_empty() || name.len() > 80 {
        return Err(RcloneError::InvalidName(
            "share name must be 1–80 characters".into(),
        ));
    }
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return Err(RcloneError::InvalidName("share name is empty".into()));
    };
    if !first.is_ascii_alphanumeric() {
        return Err(RcloneError::InvalidName(
            "share name must start with a letter or digit".into(),
        ));
    }
    if !chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-')) {
        return Err(RcloneError::InvalidName(
            "share name may only contain letters, digits, '.', '_' and '-'".into(),
        ));
    }
    Ok(())
}

fn validate_share_path(path: &str) -> Result<(), RcloneError> {
    if path.is_empty() {
        return Err(RcloneError::InvalidPath("share path is empty".into()));
    }
    if path
        .chars()
        .any(|c| c.is_control() || matches!(c, '\n' | '\r' | '"' | '#' | '='))
    {
        return Err(RcloneError::InvalidPath(
            "share path contains characters that would break rclone.conf".into(),
        ));
    }
    Ok(())
}

fn validate_and_canonicalize_path(path: &str) -> Result<String, RcloneError> {
    validate_share_path(path)?;
    if !Path::new(path).exists() {
        return Err(RcloneError::PathNotFound(path.to_string()));
    }
    let canonical =
        std::fs::canonicalize(path).map_err(|_| RcloneError::PathNotFound(path.to_string()))?;
    if !canonical.starts_with("/fs/") {
        return Err(RcloneError::PathNotInFilesystem(path.to_string()));
    }
    Ok(canonical.to_string_lossy().into_owned())
}

fn validate_listen(listen: &str) -> Result<(), RcloneError> {
    if listen.is_empty()
        || listen.chars().any(|c| {
            c.is_whitespace() || c.is_control() || matches!(c, '/' | '\\' | '"' | '\'' | ';')
        })
    {
        return Err(RcloneError::InvalidSettings(
            "listen address contains invalid characters".into(),
        ));
    }
    Ok(())
}

fn validate_port(kind: RcloneKind, port: u16) -> Result<(), RcloneError> {
    if port == 0 {
        return Err(RcloneError::InvalidSettings("port must be 1–65535".into()));
    }
    // Ports already claimed by other NASty services / the host SSH daemon.
    const RESERVED: &[u16] = &[22, 80, 443, 139, 445, 2049, 2138, 3260, 3493, 4420, 8000];
    if RESERVED.contains(&port) {
        return Err(RcloneError::InvalidSettings(format!(
            "port {port} is reserved by another NASty service"
        )));
    }
    if kind == RcloneKind::Sftp && port == 21 {
        return Err(RcloneError::InvalidSettings(
            "SFTP cannot bind FTP port 21".into(),
        ));
    }
    Ok(())
}

fn validate_username(username: &str) -> Result<(), RcloneError> {
    if username.is_empty() || username.len() > 64 {
        return Err(RcloneError::InvalidSettings(
            "username must be 1–64 characters".into(),
        ));
    }
    if !username
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-' | '@'))
    {
        return Err(RcloneError::InvalidSettings(
            "username may only contain letters, digits, '.', '_', '-' and '@'".into(),
        ));
    }
    Ok(())
}

fn validate_password(password: &str) -> Result<(), RcloneError> {
    if password.is_empty() || password.len() > 128 {
        return Err(RcloneError::InvalidSettings(
            "password must be 1–128 characters".into(),
        ));
    }
    if password.chars().any(|c| c.is_control() || c == ',') {
        return Err(RcloneError::InvalidSettings(
            "password must not contain commas or control characters".into(),
        ));
    }
    Ok(())
}

fn validate_passive_range(min: u16, max: u16) -> Result<(), RcloneError> {
    if min < 1024 || max < 1024 {
        return Err(RcloneError::InvalidSettings(
            "FTP passive ports must be ≥ 1024".into(),
        ));
    }
    if min > max {
        return Err(RcloneError::InvalidSettings(
            "FTP passive port min must be ≤ max".into(),
        ));
    }
    Ok(())
}

fn remote_id(share: &RcloneShare) -> String {
    format!("s{}", share.id.replace('-', ""))
}

/// Render rclone.conf. Enabled shares become named remotes combined under
/// `[shares]`. Read-only shares wrap the local path in a union `:ro` upstream.
pub(crate) fn render_rclone_conf(shares: &[RcloneShare]) -> String {
    let mut out = String::from("# Managed by NASty — do not edit\n\n");
    let mut upstreams = Vec::new();
    for share in shares.iter().filter(|s| s.enabled) {
        let id = remote_id(share);
        if share.read_only {
            let local = format!("{id}_local");
            out.push_str(&format!(
                "[{local}]\ntype = alias\nremote = {}\n\n[{id}]\ntype = union\nupstreams = {local}::ro\n\n",
                share.path
            ));
        } else {
            out.push_str(&format!(
                "[{id}]\ntype = alias\nremote = {}\n\n",
                share.path
            ));
        }
        upstreams.push(format!("{}={id}:", share.name));
    }
    out.push_str("[shares]\ntype = combine\n");
    if upstreams.is_empty() {
        out.push_str("upstreams =\n");
    } else {
        out.push_str("upstreams = ");
        out.push_str(&upstreams.join(" "));
        out.push('\n');
    }
    out
}

pub(crate) fn render_serve_script(
    kind: RcloneKind,
    settings: &RcloneSettings,
    shares: &[RcloneShare],
) -> String {
    let addr = format!("{}:{}", settings.listen, settings.port);
    let has_shares = shares.iter().any(|s| s.enabled);
    let remote = if has_shares {
        "shares:".to_string()
    } else {
        EMPTY_DIR.to_string()
    };
    let conf = kind.conf_path();
    let mut args: Vec<String> = vec![
        "serve".into(),
        kind.name().into(),
        remote,
        "--config".into(),
        conf,
        "--addr".into(),
        addr,
    ];
    match kind {
        RcloneKind::Ftp => {
            args.push("--passive-port".into());
            args.push(format!(
                "{}-{}",
                settings.passive_port_min, settings.passive_port_max
            ));
            if settings.anonymous {
                args.push("--user".into());
                args.push("anonymous".into());
            } else {
                args.push("--user".into());
                args.push(settings.username.clone());
                args.push("--pass".into());
                args.push(settings.password.clone());
            }
        }
        RcloneKind::Sftp => {
            args.push("--user".into());
            args.push(settings.username.clone());
            args.push("--pass".into());
            args.push(settings.password.clone());
            if Path::new(SFTP_HOST_KEY).exists() {
                args.push("--key".into());
                args.push(SFTP_HOST_KEY.into());
            }
        }
        RcloneKind::S3 => {
            args.push("--force-path-style".into());
            args.push("--auth-key".into());
            args.push(format!("{},{}", settings.username, settings.password));
        }
    }

    let mut script = String::from("#!/bin/sh\nexec /usr/bin/rclone");
    for arg in &args {
        script.push(' ');
        script.push_str(&shell_single_quote(arg));
    }
    script.push('\n');
    script
}

fn shell_single_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn share(name: &str, path: &str, read_only: bool) -> RcloneShare {
        RcloneShare {
            id: "11111111-2222-3333-4444-555555555555".into(),
            name: name.into(),
            path: path.into(),
            comment: None,
            read_only,
            enabled: true,
        }
    }

    fn settings() -> RcloneSettings {
        RcloneSettings {
            listen: "0.0.0.0".into(),
            port: 21,
            username: "nasty".into(),
            password: "s3cret".into(),
            anonymous: false,
            passive_port_min: 30000,
            passive_port_max: 30100,
        }
    }

    #[test]
    fn validate_share_name_accepts_safe_names() {
        assert!(validate_share_name("docs").is_ok());
        assert!(validate_share_name("My-Share_2024").is_ok());
        assert!(validate_share_name("a.b").is_ok());
    }

    #[test]
    fn validate_share_name_rejects_invalid() {
        assert!(validate_share_name("").is_err());
        assert!(validate_share_name(&"x".repeat(81)).is_err());
        assert!(validate_share_name("My Share").is_err());
        assert!(validate_share_name("a/b").is_err());
        assert!(validate_share_name("-leading").is_err());
        assert!(validate_share_name("has=eq").is_err());
    }

    #[test]
    fn validate_share_path_rejects_ini_injection() {
        assert!(validate_share_path("/fs/tank/docs").is_ok());
        assert!(validate_share_path("/fs/tank\nupstreams = evil").is_err());
        assert!(validate_share_path("/fs/tank#comment").is_err());
        assert!(validate_share_path("/fs/tank=eq").is_err());
    }

    #[test]
    fn render_conf_combine_and_readonly_union() {
        let rw = share("photos", "/fs/tank/photos", false);
        let mut ro = share("docs", "/fs/tank/docs", true);
        ro.id = "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee".into();
        let conf = render_rclone_conf(&[rw, ro]);
        assert!(conf.contains("[shares]"));
        assert!(conf.contains("type = combine"));
        assert!(conf.contains("photos=s11111111222233334444555555555555:"));
        assert!(conf.contains("docs=saaaaaaaabbbbccccddddeeeeeeeeeeee:"));
        assert!(conf.contains("type = union"));
        assert!(conf.contains("upstreams = saaaaaaaabbbbccccddddeeeeeeeeeeee_local::ro"));
        assert!(conf.contains("remote = /fs/tank/photos"));
    }

    #[test]
    fn render_conf_omits_disabled_shares() {
        let mut s = share("photos", "/fs/tank/photos", false);
        s.enabled = false;
        let conf = render_rclone_conf(&[s]);
        assert!(conf.contains("upstreams =\n"));
        assert!(!conf.contains("photos="));
    }

    #[test]
    fn serve_script_quotes_password_and_sets_ftp_flags() {
        let script = render_serve_script(
            RcloneKind::Ftp,
            &settings(),
            &[share("photos", "/fs/tank/photos", false)],
        );
        assert!(script.starts_with("#!/bin/sh\nexec /usr/bin/rclone"));
        assert!(script.contains("'serve' 'ftp' 'shares:'"));
        assert!(script.contains("'--addr' '0.0.0.0:21'"));
        assert!(script.contains("'--user' 'nasty'"));
        assert!(script.contains("'--pass' 's3cret'"));
        assert!(script.contains("'--passive-port' '30000-30100'"));
    }

    #[test]
    fn serve_script_anonymous_ftp_omits_password() {
        let mut s = settings();
        s.anonymous = true;
        let script = render_serve_script(RcloneKind::Ftp, &s, &[]);
        assert!(script.contains("'--user' 'anonymous'"));
        assert!(!script.contains("'--pass'"));
        assert!(script.contains(EMPTY_DIR));
    }

    #[test]
    fn serve_script_s3_auth_key_and_path_style() {
        let mut s = settings();
        s.port = 9000;
        let script =
            render_serve_script(RcloneKind::S3, &s, &[share("docs", "/fs/tank/docs", false)]);
        assert!(script.contains("'serve' 's3' 'shares:'"));
        assert!(script.contains("'--force-path-style'"));
        assert!(script.contains("'--auth-key' 'nasty,s3cret'"));
        assert!(script.contains("'--addr' '0.0.0.0:9000'"));
    }

    #[test]
    fn serve_script_escapes_single_quotes_in_password() {
        let mut s = settings();
        s.password = "it's-secret".into();
        let script = render_serve_script(RcloneKind::Sftp, &s, &[]);
        assert!(script.contains("'it'\\''s-secret'"));
    }

    #[test]
    fn reserved_ports_are_rejected() {
        assert!(validate_port(RcloneKind::Ftp, 21).is_ok());
        assert!(validate_port(RcloneKind::Ftp, 2121).is_ok());
        assert!(validate_port(RcloneKind::Sftp, 2022).is_ok());
        assert!(validate_port(RcloneKind::S3, 9000).is_ok());
        assert!(validate_port(RcloneKind::Ftp, 445).is_err());
        assert!(validate_port(RcloneKind::Sftp, 22).is_err());
        assert!(validate_port(RcloneKind::S3, 80).is_err());
    }

    #[test]
    fn shell_quote_wraps_plain_values() {
        assert_eq!(shell_single_quote("abc"), "'abc'");
        assert_eq!(shell_single_quote("a'b"), "'a'\\''b'");
    }
}
