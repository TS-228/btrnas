use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::process::{ExitStatus, Stdio};
use std::time::Duration;

use nasty_common::{HasId, StateDir};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tracing::info;
use uuid::Uuid;

const KSMBD_CONF_PATH: &str = "/etc/ksmbd/ksmbd.conf";
const STATE_DIR: &str = "/var/lib/nasty/shares/smb";
/// Usernames we registered in ksmbdpwd.db (ksmbd has no list CLI).
const KSMBD_USERS_STATE: &str = "/var/lib/nasty/shares/ksmbd-users.json";

#[derive(Debug, Error)]
pub enum SmbError {
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
    #[error("Time Machine shares are not supported with ksmbd")]
    TimeMachineUnsupported,
    #[error("ksmbd reload failed: {0}")]
    ReloadFailed(String),
    #[error("principal lookup failed: {0}")]
    PrincipalLookup(String),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SmbShare {
    /// Unique share identifier (UUID).
    pub id: String,
    /// SMB share name used in `\\server\name` UNC paths.
    pub name: String,
    /// Absolute filesystem path being shared (must be under `/fs/`).
    pub path: String,
    /// Optional description shown in share listings.
    pub comment: Option<String>,
    /// Whether the share is read-only.
    pub read_only: bool,
    /// Whether the share is visible in network browse lists.
    pub browseable: bool,
    /// Whether unauthenticated guest access is allowed.
    pub guest_ok: bool,
    /// Usernames allowed to connect (empty means no restriction beyond authentication).
    pub valid_users: Vec<String>,
    /// Additional raw ksmbd share parameters written to the share section.
    pub extra_params: HashMap<String, String>,
    /// Legacy Time Machine flag. Always rejected on create/update — ksmbd
    /// has no vfs_fruit / Time Machine support.
    #[serde(default)]
    pub time_machine: bool,
    /// Optional Time Machine size cap in GiB, written as
    /// `fruit:time machine max size` so macOS self-limits and thins old
    /// backups. `None` = no advertised cap (pair with a subvolume quota).
    #[serde(default)]
    pub time_machine_max_size_gib: Option<u32>,
    /// Whether the share is active in `ksmbd.conf`.
    pub enabled: bool,
}

impl HasId for SmbShare {
    fn id(&self) -> &str {
        &self.id
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct CreateSmbShareRequest {
    /// SMB share name (1–80 characters, no special characters).
    pub name: String,
    /// Absolute path to share (must exist and be under `/fs/`).
    pub path: String,
    /// Optional description.
    pub comment: Option<String>,
    /// Whether the share is read-only (default: false).
    pub read_only: Option<bool>,
    /// Whether the share appears in browse lists (default: true).
    pub browseable: Option<bool>,
    /// Whether guest access is allowed (default: false).
    pub guest_ok: Option<bool>,
    /// Allowed usernames; empty means no per-user restriction.
    pub valid_users: Option<Vec<String>>,
    /// Additional raw ksmbd parameters for this share section.
    pub extra_params: Option<HashMap<String, String>>,
    /// Make this a macOS Time Machine destination (default: false). Requires
    /// an authenticated, writable share.
    pub time_machine: Option<bool>,
    /// Optional Time Machine size cap in GiB.
    pub time_machine_max_size_gib: Option<u32>,
    /// Whether to enable the share immediately (default: true).
    pub enabled: Option<bool>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct UpdateSmbShareRequest {
    /// ID of the share to update.
    pub id: String,
    /// New share name (optional; must be unique).
    pub name: Option<String>,
    /// New description (optional).
    pub comment: Option<String>,
    /// Update read-only flag (optional).
    pub read_only: Option<bool>,
    /// Update browseable flag (optional).
    pub browseable: Option<bool>,
    /// Update guest access flag (optional).
    pub guest_ok: Option<bool>,
    /// Replacement allowed-users list (optional).
    pub valid_users: Option<Vec<String>>,
    /// Replacement extra ksmbd parameters (optional).
    pub extra_params: Option<HashMap<String, String>>,
    /// Toggle Time Machine destination (optional).
    pub time_machine: Option<bool>,
    /// Update the Time Machine size cap in GiB (optional). Send 0 to clear.
    pub time_machine_max_size_gib: Option<u32>,
    /// Enable or disable the share (optional).
    pub enabled: Option<bool>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct DeleteSmbShareRequest {
    pub id: String,
}

fn state_dir() -> StateDir {
    StateDir::new(STATE_DIR)
}

pub struct SmbService;

/// Current NSS/winbind view of a portal user's identity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrincipalAuthorization {
    pub principal: String,
    pub groups: Vec<String>,
}

impl Default for SmbService {
    fn default() -> Self {
        Self::new()
    }
}

impl SmbService {
    pub fn new() -> Self {
        Self
    }

    /// Ensure `/etc/ksmbd/ksmbd.conf` exists with globals + enabled shares.
    /// Idempotent; safe to run on every boot before starting ksmbd.
    pub async fn ensure_config_scaffolding(&self) -> Result<(), SmbError> {
        rewrite_ksmbd_conf().await
    }

    pub async fn list(&self) -> Result<Vec<SmbShare>, SmbError> {
        Ok(state_dir().load_all().await)
    }

    /// List all shares, failing instead of silently omitting unreadable state.
    pub async fn list_strict(&self) -> Result<Vec<SmbShare>, SmbError> {
        Ok(state_dir().load_all_strict().await?)
    }

    pub async fn get(&self, id: &str) -> Result<SmbShare, SmbError> {
        state_dir()
            .load::<SmbShare>(id)
            .await
            .ok_or_else(|| SmbError::NotFound(id.to_string()))
    }

    /// Resolve a local or domain principal and all of its current groups via
    /// NSS/winbind. Commands receive the principal as a single argv value;
    /// numeric GIDs are resolved separately so group names containing spaces
    /// are never split on whitespace.
    pub async fn principal_authorization(
        &self,
        principal: &str,
    ) -> Result<PrincipalAuthorization, SmbError> {
        validate_file_principal(principal)?;
        tokio::time::timeout(
            Duration::from_secs(5),
            resolve_principal_authorization(principal),
        )
        .await
        .map_err(|_| SmbError::PrincipalLookup("lookup timed out".to_string()))?
    }

    pub async fn create(&self, req: CreateSmbShareRequest) -> Result<SmbShare, SmbError> {
        validate_share_name(&req.name)?;
        validate_share_path(&req.path)?;
        if let Some(ref valid_users) = req.valid_users {
            validate_valid_users(valid_users)?;
        }

        if !Path::new(&req.path).exists() {
            return Err(SmbError::PathNotFound(req.path));
        }
        let canonical = std::fs::canonicalize(&req.path)
            .map_err(|_| SmbError::PathNotFound(req.path.clone()))?;
        if !canonical.starts_with("/fs/") {
            return Err(SmbError::PathNotInFilesystem(req.path));
        }

        let shares: Vec<SmbShare> = state_dir().load_all().await;

        if let Some(existing) = shares.into_iter().find(|s| s.name == req.name) {
            info!(
                "SMB share '{}' already exists, returning existing (idempotent)",
                req.name
            );
            return Ok(existing);
        }

        let share = SmbShare {
            id: Uuid::new_v4().to_string(),
            name: req.name,
            path: req.path,
            comment: req.comment,
            read_only: req.read_only.unwrap_or(false),
            browseable: req.browseable.unwrap_or(true),
            guest_ok: req.guest_ok.unwrap_or(false),
            valid_users: req.valid_users.unwrap_or_default(),
            extra_params: req.extra_params.unwrap_or_default(),
            time_machine: req.time_machine.unwrap_or(false),
            time_machine_max_size_gib: req.time_machine_max_size_gib.filter(|&n| n > 0),
            enabled: req.enabled.unwrap_or(true),
        };
        reject_time_machine(&share)?;

        state_dir().save(&share.id, &share).await?;
        rewrite_ksmbd_conf().await?;
        reload_ksmbd().await?;
        wait_for_share_ready(&share.name).await;

        info!("Created SMB share '{}' at {}", share.name, share.path);
        Ok(share)
    }

    pub async fn update(&self, req: UpdateSmbShareRequest) -> Result<SmbShare, SmbError> {
        if let Some(ref new_name) = req.name {
            validate_share_name(new_name)?;
        }

        let mut share: SmbShare = state_dir()
            .load(&req.id)
            .await
            .ok_or_else(|| SmbError::NotFound(req.id.clone()))?;

        // Check name uniqueness if changing
        if let Some(ref new_name) = req.name {
            let shares: Vec<SmbShare> = state_dir().load_all().await;
            if shares.iter().any(|s| s.name == *new_name && s.id != req.id) {
                return Err(SmbError::NameExists(new_name.clone()));
            }
        }

        if let Some(name) = req.name {
            share.name = name;
        }
        if let Some(comment) = req.comment {
            share.comment = Some(comment);
        }
        if let Some(read_only) = req.read_only {
            share.read_only = read_only;
        }
        if let Some(browseable) = req.browseable {
            share.browseable = browseable;
        }
        if let Some(guest_ok) = req.guest_ok {
            share.guest_ok = guest_ok;
        }
        if let Some(valid_users) = req.valid_users {
            validate_valid_users(&valid_users)?;
            share.valid_users = valid_users;
        }
        if let Some(extra_params) = req.extra_params {
            share.extra_params = extra_params;
        }
        if let Some(time_machine) = req.time_machine {
            share.time_machine = time_machine;
        }
        if let Some(n) = req.time_machine_max_size_gib {
            // 0 clears the cap.
            share.time_machine_max_size_gib = if n > 0 { Some(n) } else { None };
        }
        if let Some(enabled) = req.enabled {
            share.enabled = enabled;
        }
        reject_time_machine(&share)?;

        state_dir().save(&share.id, &share).await?;
        rewrite_ksmbd_conf().await?;
        reload_ksmbd().await?;

        info!("Updated SMB share '{}'", share.name);
        Ok(share)
    }

    pub async fn delete(&self, req: DeleteSmbShareRequest) -> Result<(), SmbError> {
        let _: SmbShare = state_dir()
            .load(&req.id)
            .await
            .ok_or_else(|| SmbError::NotFound(req.id.clone()))?;

        state_dir().remove(&req.id).await?;
        rewrite_ksmbd_conf().await?;
        reload_ksmbd().await?;

        info!("Deleted SMB share '{}'", req.id);
        Ok(())
    }
}

/// Pure portal policy. SMB principal names are ASCII case-insensitive, but
/// matches remain exact so similarly named users and groups cannot collide.
pub fn share_allows_principal(share: &SmbShare, principal: &str, groups: &[String]) -> bool {
    // Raw directives are evaluated by ksmbd and can override path, valid
    // users, invalid users, and access semantics. The portal cannot safely
    // reproduce that effective policy, so any such share is portal-ineligible.
    if !share.enabled
        || share.guest_ok
        || share.valid_users.is_empty()
        || !share.extra_params.is_empty()
    {
        return false;
    }

    share.valid_users.iter().any(|allowed| {
        if let Some(group) = allowed.strip_prefix('@') {
            groups
                .iter()
                .any(|current| current.eq_ignore_ascii_case(group))
        } else {
            allowed.eq_ignore_ascii_case(principal)
        }
    })
}

fn validate_file_principal(principal: &str) -> Result<(), SmbError> {
    if principal.is_empty() || principal.trim() != principal || principal.starts_with('@') {
        return Err(SmbError::InvalidName(
            "file principal must be a non-empty user principal".to_string(),
        ));
    }
    validate_valid_users(&[principal.to_string()])
}

const MAX_NSS_OUTPUT: usize = 64 * 1024;
const MAX_PRINCIPAL_GROUPS: usize = 128;

struct BoundedOutput {
    status: ExitStatus,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
}

async fn resolve_principal_authorization(
    principal: &str,
) -> Result<PrincipalAuthorization, SmbError> {
    let output = run_bounded_command("id", &["-G", "--", principal]).await?;
    if !output.status.success() {
        return Err(SmbError::PrincipalLookup(format!(
            "principal does not exist: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    let gids = std::str::from_utf8(&output.stdout)
        .map_err(|_| SmbError::PrincipalLookup("id returned non-UTF-8 output".to_string()))?;
    let mut unique_gids = HashSet::new();
    let gids: Vec<String> = gids
        .split_whitespace()
        .map(|gid| {
            gid.parse::<u32>()
                .map(|_| gid.to_string())
                .map_err(|_| SmbError::PrincipalLookup("id returned an invalid GID".to_string()))
        })
        .collect::<Result<_, _>>()?;
    let gids: Vec<String> = gids
        .into_iter()
        .filter(|gid| unique_gids.insert(gid.clone()))
        .collect();
    if gids.is_empty() || gids.len() > MAX_PRINCIPAL_GROUPS {
        return Err(SmbError::PrincipalLookup(
            "principal has no groups or exceeds the group limit".to_string(),
        ));
    }

    let mut groups = Vec::with_capacity(gids.len());
    for gid in gids {
        let output = run_bounded_command("getent", &["group", &gid]).await?;
        if !output.status.success() {
            return Err(SmbError::PrincipalLookup(format!(
                "could not resolve GID {gid}"
            )));
        }
        let record = std::str::from_utf8(&output.stdout).map_err(|_| {
            SmbError::PrincipalLookup("getent returned non-UTF-8 output".to_string())
        })?;
        let name = record
            .lines()
            .next()
            .and_then(|line| line.split_once(':').map(|(name, _)| name))
            .filter(|name| !name.is_empty())
            .ok_or_else(|| {
                SmbError::PrincipalLookup(format!(
                    "getent returned an invalid record for GID {gid}"
                ))
            })?;
        groups.push(name.to_string());
    }

    // Local portal principals must exist in both NSS and ksmbdpwd.db.
    // Domain principals are not supported with ksmbd on this fork.
    if principal.contains('\\') {
        return Err(SmbError::PrincipalLookup(
            "domain principals are not supported with ksmbd".to_string(),
        ));
    }
    let users = load_ksmbd_user_names().await?;
    if !users.iter().any(|u| u.eq_ignore_ascii_case(principal)) {
        return Err(SmbError::PrincipalLookup(
            "local principal is not present in ksmbd user database".to_string(),
        ));
    }

    Ok(PrincipalAuthorization {
        principal: principal.to_string(),
        groups,
    })
}

async fn load_ksmbd_user_names() -> Result<Vec<String>, SmbError> {
    match tokio::fs::read_to_string(KSMBD_USERS_STATE).await {
        Ok(raw) => serde_json::from_str(&raw)
            .map_err(|e| SmbError::ReloadFailed(format!("parse {KSMBD_USERS_STATE}: {e}"))),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(e) => Err(SmbError::Io(e)),
    }
}

async fn save_ksmbd_user_names(users: &[String]) -> Result<(), SmbError> {
    if let Some(parent) = Path::new(KSMBD_USERS_STATE).parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    let raw = serde_json::to_string_pretty(users)
        .map_err(|e| SmbError::ReloadFailed(format!("serialize ksmbd users: {e}")))?;
    tokio::fs::write(KSMBD_USERS_STATE, raw).await?;
    Ok(())
}

async fn run_bounded_command(program: &str, args: &[&str]) -> Result<BoundedOutput, SmbError> {
    use tokio::io::AsyncReadExt;

    let mut child = tokio::process::Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|error| SmbError::PrincipalLookup(format!("start {program}: {error}")))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| SmbError::PrincipalLookup(format!("capture {program} stdout")))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| SmbError::PrincipalLookup(format!("capture {program} stderr")))?;

    let collect = async {
        let read_stdout = async {
            let mut bytes = Vec::new();
            stdout
                .take((MAX_NSS_OUTPUT + 1) as u64)
                .read_to_end(&mut bytes)
                .await
                .map(|_| bytes)
        };
        let read_stderr = async {
            let mut bytes = Vec::new();
            stderr
                .take((MAX_NSS_OUTPUT + 1) as u64)
                .read_to_end(&mut bytes)
                .await
                .map(|_| bytes)
        };
        let (stdout, stderr, status) = tokio::join!(read_stdout, read_stderr, child.wait());
        let stdout = stdout.map_err(|error| {
            SmbError::PrincipalLookup(format!("read {program} stdout: {error}"))
        })?;
        let stderr = stderr.map_err(|error| {
            SmbError::PrincipalLookup(format!("read {program} stderr: {error}"))
        })?;
        let status = status
            .map_err(|error| SmbError::PrincipalLookup(format!("wait for {program}: {error}")))?;
        if stdout.len() > MAX_NSS_OUTPUT || stderr.len() > MAX_NSS_OUTPUT {
            return Err(SmbError::PrincipalLookup(format!(
                "{program} output exceeded the limit"
            )));
        }
        Ok(BoundedOutput {
            status,
            stdout,
            stderr,
        })
    };

    tokio::time::timeout(Duration::from_secs(2), collect)
        .await
        .map_err(|_| SmbError::PrincipalLookup(format!("{program} timed out")))?
}

/// Strip characters that could inject new ksmbd.conf directives.
/// Removes newlines, carriage returns, semicolons, and other control characters.
fn sanitize_smb_value(s: &str) -> String {
    s.chars()
        .filter(|c| !c.is_control() && *c != ';' && *c != '\n' && *c != '\r')
        .collect()
}

fn validate_share_name(name: &str) -> Result<(), SmbError> {
    if name.is_empty()
        || name.len() > 80
        || name.contains([
            '/', '\\', '[', ']', ':', '|', '<', '>', '+', '=', ';', ',', '?', '*',
        ])
    {
        return Err(SmbError::InvalidName(
            "Share name must be 1-80 chars without special characters".to_string(),
        ));
    }
    Ok(())
}

/// Validate a share path before it's written into smb.conf as
/// `path = <path>`. The create/update flow also canonicalizes and
/// requires the path live under `/fs/`, so this is defense-in-depth:
/// reject characters that, if present in a real filesystem entry name,
/// would smuggle a new `include = /etc/passwd` directive into the
/// rendered share section. Newlines and carriage returns are the
/// primary concern; the rest are belt-and-braces against weird
/// filesystem entries (most are syntactically invalid in smb.conf
/// values anyway but we'd rather fail loudly than render garbage).
fn validate_share_path(path: &str) -> Result<(), SmbError> {
    if path.is_empty() {
        return Err(SmbError::InvalidName("share path is empty".to_string()));
    }
    if path
        .chars()
        .any(|c| c == '\n' || c == '\r' || c == '\0' || c == '"' || c == '#')
    {
        return Err(SmbError::InvalidName(
            "share path contains characters that would inject smb.conf directives \
             (newline, CR, NUL, double-quote, '#')"
                .to_string(),
        ));
    }
    Ok(())
}

/// Validate share `valid_users` entries. Three accepted shapes:
/// local `name`, local `@group`, and domain `DOMAIN\name` (optionally
/// `@DOMAIN\group`) — the backslash carve-out for AD member mode.
/// Everything is checked against config-injection characters the same
/// way validate_share_path is; the domain part follows NetBIOS rules
/// (≤15 chars, alphanumeric + hyphen).
fn validate_valid_users(entries: &[String]) -> Result<(), SmbError> {
    for raw in entries {
        let entry = raw.strip_prefix('@').unwrap_or(raw);
        if entry.is_empty() || entry.len() > 256 {
            return Err(SmbError::InvalidName(format!(
                "invalid valid_users entry '{raw}'"
            )));
        }
        if entry
            .chars()
            .any(|c| c.is_control() || matches!(c, ';' | '"' | '#' | '=' | '\n' | '\r'))
        {
            return Err(SmbError::InvalidName(format!(
                "valid_users entry '{raw}' contains forbidden characters"
            )));
        }
        match entry.split('\\').collect::<Vec<_>>().as_slice() {
            // Local user or group: existing character policy.
            [name] => {
                if !name
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
                {
                    return Err(SmbError::InvalidName(format!(
                        "invalid user/group name '{raw}'"
                    )));
                }
            }
            // DOMAIN\name: NetBIOS domain + AD account (spaces/dots legal).
            [domain, name] => {
                let domain_ok = !domain.is_empty()
                    && domain.len() <= 15
                    && domain
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || c == '-');
                let name_ok = !name.is_empty()
                    && name
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | ' '));
                if !domain_ok || !name_ok {
                    return Err(SmbError::InvalidName(format!(
                        "invalid domain principal '{raw}'"
                    )));
                }
            }
            _ => {
                return Err(SmbError::InvalidName(format!(
                    "invalid valid_users entry '{raw}'"
                )));
            }
        }
    }
    Ok(())
}

/// Optional SMB tuning knobs merged into `[global]` (from TuningService).
#[derive(Debug, Clone, Default)]
pub struct SmbTuningGlobals {
    pub max_connections: u32,
    pub deadtime: u32,
}

/// Render a single share config section. Pure: no I/O.
fn render_share_conf(share: &SmbShare) -> String {
    let mut conf = format!("[{}]\n", sanitize_smb_value(&share.name));
    conf.push_str(&format!("    path = {}\n", share.path));

    if let Some(ref comment) = share.comment {
        conf.push_str(&format!("    comment = {}\n", sanitize_smb_value(comment)));
    }

    conf.push_str(&format!(
        "    read only = {}\n",
        if share.read_only { "yes" } else { "no" }
    ));
    conf.push_str(&format!(
        "    browseable = {}\n",
        if share.browseable { "yes" } else { "no" }
    ));
    conf.push_str(&format!(
        "    guest ok = {}\n",
        if share.guest_ok { "yes" } else { "no" }
    ));

    if share.guest_ok {
        conf.push_str("    force user = nobody\n");
        conf.push_str("    force group = nogroup\n");
        conf.push_str("    create mask = 0666\n");
        conf.push_str("    directory mask = 0777\n");
    } else if !share.valid_users.is_empty() {
        if let Some(first_user) = share.valid_users.iter().find(|u| !u.starts_with('@')) {
            conf.push_str(&format!(
                "    force user = {}\n",
                sanitize_smb_value(first_user)
            ));
        }
        conf.push_str("    create mask = 0664\n");
        conf.push_str("    directory mask = 0775\n");
    }

    if !share.valid_users.is_empty() {
        let sanitized_users: Vec<String> = share
            .valid_users
            .iter()
            .map(|u| {
                let s = sanitize_smb_value(u);
                if s.contains(' ') {
                    format!("\"{s}\"")
                } else {
                    s
                }
            })
            .collect();
        conf.push_str(&format!(
            "    valid users = {}\n",
            sanitized_users.join(" ")
        ));
    }

    let mut extra: Vec<_> = share.extra_params.iter().collect();
    extra.sort_by_key(|(k, _)| *k);
    for (key, value) in extra {
        conf.push_str(&format!(
            "    {} = {}\n",
            sanitize_smb_value(key),
            sanitize_smb_value(value)
        ));
    }

    conf
}

fn reject_time_machine(share: &SmbShare) -> Result<(), SmbError> {
    if share.time_machine || share.time_machine_max_size_gib.is_some() {
        return Err(SmbError::TimeMachineUnsupported);
    }
    Ok(())
}

/// Build full ksmbd.conf contents: `[global]` + enabled share sections.
fn render_ksmbd_conf(shares: &[SmbShare], tuning: &SmbTuningGlobals) -> String {
    let mut conf = String::from("# Managed by NASty — do not edit manually\n\n");
    conf.push_str("[global]\n");
    conf.push_str("    workgroup = WORKGROUP\n");
    conf.push_str("    server string = NASty\n");
    conf.push_str("    map to guest = bad user\n");
    conf.push_str("    guest account = nobody\n");
    if tuning.max_connections > 0 {
        conf.push_str(&format!(
            "    max connections = {}\n",
            tuning.max_connections
        ));
    }
    if tuning.deadtime > 0 {
        conf.push_str(&format!("    deadtime = {}\n", tuning.deadtime));
    }
    conf.push('\n');

    let mut enabled: Vec<&SmbShare> = shares.iter().filter(|s| s.enabled).collect();
    enabled.sort_by_key(|s| s.name.to_ascii_lowercase());
    for share in enabled {
        conf.push_str(&render_share_conf(share));
        conf.push('\n');
    }
    conf
}

fn load_tuning_globals_sync() -> SmbTuningGlobals {
    // Best-effort read of tuning.json so share rewrites pick up SMB knobs
    // without a circular crate dependency on nasty-system.
    let Ok(raw) = std::fs::read_to_string("/var/lib/nasty/tuning.json") else {
        return SmbTuningGlobals::default();
    };
    let Ok(v) = serde_json::from_str::<serde_json::Value>(&raw) else {
        return SmbTuningGlobals::default();
    };
    SmbTuningGlobals {
        max_connections: v
            .get("smb_max_connections")
            .and_then(|x| x.as_u64())
            .unwrap_or(0) as u32,
        deadtime: v.get("smb_deadtime").and_then(|x| x.as_u64()).unwrap_or(0) as u32,
    }
}

async fn rewrite_ksmbd_conf() -> Result<(), SmbError> {
    if let Some(parent) = Path::new(KSMBD_CONF_PATH).parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    let shares: Vec<SmbShare> = state_dir().load_all().await;
    let tuning = load_tuning_globals_sync();
    let conf = render_ksmbd_conf(&shares, &tuning);
    tokio::fs::write(KSMBD_CONF_PATH, &conf).await?;

    for share in shares.iter().filter(|s| s.enabled) {
        // ksmbd maps to UNIX users; keep share trees world-writable so
        // ACL is enforced by valid_users / guest ok rather than mode bits.
        nasty_common::cmd::try_run("chmod", &["0777", &share.path]).await;
    }
    Ok(())
}

/// Wait for an SMB share to appear in `ksmbd.control --list` after reload.
async fn wait_for_share_ready(share_name: &str) {
    for attempt in 1..=10 {
        let output = tokio::process::Command::new("ksmbd.control")
            .args(["--list"])
            .stderr(std::process::Stdio::null())
            .output()
            .await;
        if let Ok(out) = output {
            let stdout = String::from_utf8_lossy(&out.stdout);
            if stdout
                .lines()
                .any(|l| l.trim().eq_ignore_ascii_case(share_name))
            {
                info!("SMB share '{share_name}' is ready (attempt {attempt})");
                return;
            }
        }
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    }
    tracing::warn!("SMB share '{share_name}' readiness check timed out — proceeding anyway");
}

async fn reload_ksmbd() -> Result<(), SmbError> {
    let output = tokio::process::Command::new("ksmbd.control")
        .args(["--reload"])
        .output()
        .await?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        // Not running yet is fine — next start picks up the file.
        if stderr.contains("No such process")
            || stderr.contains("not running")
            || stderr.to_ascii_lowercase().contains("no ksmbd")
        {
            info!("ksmbd not running; conf written for next start");
            return Ok(());
        }
        return Err(SmbError::ReloadFailed(stderr.to_string()));
    }

    info!("ksmbd configuration reloaded");
    Ok(())
}

// ── SMB User Management ─────────────────────────────────────────

const SMB_USER_UID_MIN: u32 = 3000;

/// SMB user info returned by list.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SmbUser {
    /// Linux username.
    pub username: String,
    /// Unix UID.
    pub uid: u32,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct CreateSmbUserRequest {
    /// Username (alphanumeric + hyphens, 1-32 chars).
    pub username: String,
    /// Password for SMB authentication.
    pub password: String,
}

impl SmbService {
    /// Create a Linux system user and register them in ksmbdpwd.db.
    pub async fn create_user(&self, req: CreateSmbUserRequest) -> Result<SmbUser, SmbError> {
        let username = req.username.trim();
        if username.is_empty() || username.len() > 32 {
            return Err(SmbError::InvalidName(
                "username must be 1-32 characters".into(),
            ));
        }
        if !username
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        {
            return Err(SmbError::InvalidName(
                "username must be alphanumeric, hyphens, or underscores".into(),
            ));
        }

        // Check if user already exists
        let check = tokio::process::Command::new("id")
            .arg(username)
            .output()
            .await
            .map_err(|e| SmbError::ReloadFailed(format!("id: {e}")))?;
        if check.status.success() {
            return Err(SmbError::NameExists(username.to_string()));
        }

        // Find next available UID
        let uid = next_available_uid().await;

        // Create system user with no shell, no home
        let output = tokio::process::Command::new("useradd")
            .args([
                "--system",
                "--uid",
                &uid.to_string(),
                "--no-create-home",
                "--shell",
                "/usr/sbin/nologin",
                username,
            ])
            .output()
            .await
            .map_err(|e| SmbError::ReloadFailed(format!("useradd: {e}")))?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(SmbError::ReloadFailed(format!("useradd failed: {stderr}")));
        }

        if let Err(e) = set_ksmbd_password(username, &req.password, /*add*/ true).await {
            let _ = tokio::process::Command::new("userdel")
                .arg(username)
                .output()
                .await;
            return Err(e);
        }

        let mut users = load_ksmbd_user_names().await?;
        if !users.iter().any(|u| u == username) {
            users.push(username.to_string());
            users.sort();
            save_ksmbd_user_names(&users).await?;
        }

        info!("Created SMB user '{username}' (UID {uid})");
        Ok(SmbUser {
            username: username.to_string(),
            uid,
        })
    }

    /// Delete a Linux system user and remove them from ksmbdpwd.db.
    pub async fn delete_user(&self, username: &str) -> Result<(), SmbError> {
        nasty_common::cmd::try_run("ksmbd.adduser", &["--delete", username]).await;

        let mut users = load_ksmbd_user_names().await.unwrap_or_default();
        let before = users.len();
        users.retain(|u| !u.eq_ignore_ascii_case(username));
        if users.len() != before {
            let _ = save_ksmbd_user_names(&users).await;
        }

        // Delete system user
        let output = tokio::process::Command::new("userdel")
            .arg(username)
            .output()
            .await
            .map_err(|e| SmbError::ReloadFailed(format!("userdel: {e}")))?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(SmbError::ReloadFailed(format!("userdel failed: {stderr}")));
        }

        info!("Deleted SMB user '{username}'");
        Ok(())
    }

    /// Change an SMB user's password.
    pub async fn set_user_password(&self, username: &str, password: &str) -> Result<(), SmbError> {
        set_ksmbd_password(username, password, /*add*/ false).await?;
        info!("Changed password for SMB user '{username}'");
        Ok(())
    }

    /// List SMB users registered in the engine's ksmbd user state.
    pub async fn list_users(&self) -> Result<Vec<SmbUser>, SmbError> {
        let names = load_ksmbd_user_names().await?;
        let mut users = Vec::new();
        for username in names {
            let output = tokio::process::Command::new("id")
                .args(["-u", &username])
                .output()
                .await;
            let uid = match output {
                Ok(out) if out.status.success() => String::from_utf8_lossy(&out.stdout)
                    .trim()
                    .parse()
                    .unwrap_or(0),
                _ => 0,
            };
            if uid >= SMB_USER_UID_MIN {
                users.push(SmbUser { username, uid });
            }
        }
        Ok(users)
    }
}

// ── SMB Group Management ────────────────────────────────────────

const SMB_GROUP_GID_MIN: u32 = 3000;

/// SMB group info returned by list.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SmbGroup {
    /// Linux group name.
    pub name: String,
    /// Unix GID.
    pub gid: u32,
    /// Group members (usernames).
    pub members: Vec<String>,
}

impl SmbService {
    /// Create a Linux system group for SMB access control.
    pub async fn create_group(&self, name: &str) -> Result<SmbGroup, SmbError> {
        let name = name.trim();
        if name.is_empty() || name.len() > 32 {
            return Err(SmbError::InvalidName(
                "group name must be 1-32 characters".into(),
            ));
        }
        if !name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        {
            return Err(SmbError::InvalidName(
                "group name must be alphanumeric, hyphens, or underscores".into(),
            ));
        }

        // Check if group already exists
        let check = tokio::process::Command::new("getent")
            .args(["group", name])
            .output()
            .await
            .map_err(|e| SmbError::ReloadFailed(format!("getent: {e}")))?;
        if check.status.success() {
            return Err(SmbError::NameExists(name.to_string()));
        }

        let gid = next_available_gid().await;

        let output = tokio::process::Command::new("groupadd")
            .args(["--gid", &gid.to_string(), name])
            .output()
            .await
            .map_err(|e| SmbError::ReloadFailed(format!("groupadd: {e}")))?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(SmbError::ReloadFailed(format!("groupadd failed: {stderr}")));
        }

        info!("Created SMB group '{name}' (GID {gid})");
        Ok(SmbGroup {
            name: name.to_string(),
            gid,
            members: vec![],
        })
    }

    /// Delete a Linux system group.
    pub async fn delete_group(&self, name: &str) -> Result<(), SmbError> {
        let output = tokio::process::Command::new("groupdel")
            .arg(name)
            .output()
            .await
            .map_err(|e| SmbError::ReloadFailed(format!("groupdel: {e}")))?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(SmbError::ReloadFailed(format!("groupdel failed: {stderr}")));
        }
        info!("Deleted SMB group '{name}'");
        Ok(())
    }

    /// List SMB-managed groups (GIDs in the 3000+ range).
    pub async fn list_groups(&self) -> Result<Vec<SmbGroup>, SmbError> {
        let content = tokio::fs::read_to_string("/etc/group")
            .await
            .map_err(|e| SmbError::ReloadFailed(format!("read /etc/group: {e}")))?;

        let mut groups = Vec::new();
        for line in content.lines() {
            // /etc/group format: name:x:gid:member1,member2
            let parts: Vec<&str> = line.splitn(4, ':').collect();
            if parts.len() >= 3
                && let Ok(gid) = parts[2].parse::<u32>()
                && (SMB_GROUP_GID_MIN..SMB_GROUP_GID_MIN + 1000).contains(&gid)
            {
                let members = parts
                    .get(3)
                    .map(|m| {
                        m.split(',')
                            .filter(|s| !s.is_empty())
                            .map(|s| s.to_string())
                            .collect()
                    })
                    .unwrap_or_default();
                groups.push(SmbGroup {
                    name: parts[0].to_string(),
                    gid,
                    members,
                });
            }
        }
        Ok(groups)
    }

    /// Add a user to a group.
    pub async fn add_group_member(&self, group: &str, user: &str) -> Result<(), SmbError> {
        let output = tokio::process::Command::new("usermod")
            .args(["-aG", group, user])
            .output()
            .await
            .map_err(|e| SmbError::ReloadFailed(format!("usermod: {e}")))?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(SmbError::ReloadFailed(format!("usermod failed: {stderr}")));
        }
        info!("Added user '{user}' to group '{group}'");
        Ok(())
    }

    /// Remove a user from a group.
    pub async fn remove_group_member(&self, group: &str, user: &str) -> Result<(), SmbError> {
        let output = tokio::process::Command::new("gpasswd")
            .args(["-d", user, group])
            .output()
            .await
            .map_err(|e| SmbError::ReloadFailed(format!("gpasswd: {e}")))?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(SmbError::ReloadFailed(format!("gpasswd failed: {stderr}")));
        }
        info!("Removed user '{user}' from group '{group}'");
        Ok(())
    }
}

/// Find the next available GID starting from SMB_GROUP_GID_MIN.
async fn next_available_gid() -> u32 {
    for gid in SMB_GROUP_GID_MIN..SMB_GROUP_GID_MIN + 1000 {
        let check = tokio::process::Command::new("getent")
            .args(["group", &gid.to_string()])
            .output()
            .await;
        if let Ok(out) = check
            && !out.status.success()
        {
            return gid;
        }
    }
    SMB_GROUP_GID_MIN
}

/// Set ksmbd password via `ksmbd.adduser --password=…`.
async fn set_ksmbd_password(username: &str, password: &str, add: bool) -> Result<(), SmbError> {
    let action = if add { "--add" } else { "--update" };
    let pwd_arg = format!("--password={password}");
    let output = tokio::process::Command::new("ksmbd.adduser")
        .args([action, &pwd_arg, username])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .output()
        .await
        .map_err(|e| SmbError::ReloadFailed(format!("ksmbd.adduser: {e}")))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(SmbError::ReloadFailed(format!(
            "ksmbd.adduser failed: {stderr}"
        )));
    }
    Ok(())
}

/// Find the next available UID starting from SMB_USER_UID_MIN.
async fn next_available_uid() -> u32 {
    for uid in SMB_USER_UID_MIN..SMB_USER_UID_MIN + 1000 {
        let check = tokio::process::Command::new("id")
            .arg(uid.to_string())
            .output()
            .await;
        if let Ok(out) = check
            && !out.status.success()
        {
            return uid;
        }
    }
    SMB_USER_UID_MIN // fallback
}

#[cfg(test)]
mod tests {
    use super::*;

    fn minimal_share() -> SmbShare {
        SmbShare {
            id: "id-1".to_string(),
            name: "share1".to_string(),
            path: "/fs/p/data".to_string(),
            comment: None,
            read_only: false,
            browseable: true,
            guest_ok: false,
            valid_users: vec![],
            extra_params: HashMap::new(),
            time_machine: false,
            time_machine_max_size_gib: None,
            enabled: true,
        }
    }

    // ── validators / sanitizers ────────────────────────────────────

    #[test]
    fn sanitize_smb_value_strips_injection_chars() {
        assert_eq!(sanitize_smb_value("hello"), "hello");
        assert_eq!(sanitize_smb_value("a;b\nc\rd"), "abcd");
        assert_eq!(
            sanitize_smb_value("name\twith\x01control"),
            "namewithcontrol"
        );
    }

    #[test]
    fn validate_share_name_accepts_normal_names() {
        assert!(validate_share_name("docs").is_ok());
        assert!(validate_share_name("My Share 2024").is_ok());
    }

    #[test]
    fn validate_share_name_rejects_invalid() {
        assert!(validate_share_name("").is_err());
        assert!(validate_share_name(&"x".repeat(81)).is_err());
        for bad in ["a/b", "a\\b", "a[b", "a]b", "a:b", "a|b", "a;b"] {
            assert!(validate_share_name(bad).is_err(), "should reject '{bad}'");
        }
    }

    #[test]
    fn validate_share_path_accepts_real_paths() {
        assert!(validate_share_path("/fs/tank/docs").is_ok());
        assert!(validate_share_path("/fs/pool/My Files").is_ok());
        assert!(validate_share_path("/fs/a/b/c-d_e.f").is_ok());
    }

    #[test]
    fn validate_share_path_rejects_smb_conf_injection() {
        // Newline + new key=value smuggles a fresh smb.conf directive
        // into the rendered share section, e.g. an extra `include = `.
        assert!(validate_share_path("/fs/tank\ninclude = /etc/passwd").is_err());
        assert!(validate_share_path("/fs/tank\rinclude").is_err());
        assert!(validate_share_path("/fs/tank\0bad").is_err());
        // `#` starts a comment in smb.conf — a path containing it
        // would silently truncate the path line on parse.
        assert!(validate_share_path("/fs/tank#comment").is_err());
        // Double-quote could break a quoted value if the renderer
        // later switches to quoted style.
        assert!(validate_share_path("/fs/tank\"quoted").is_err());
    }

    #[test]
    fn validate_share_path_rejects_empty() {
        assert!(validate_share_path("").is_err());
    }

    #[test]
    fn validate_valid_users_accepts_local_group_and_domain_forms() {
        assert!(validate_valid_users(&["alice".into()]).is_ok());
        assert!(validate_valid_users(&["@staff".into()]).is_ok());
        assert!(validate_valid_users(&["CORP\\alice".into()]).is_ok());
        assert!(validate_valid_users(&["@CORP\\domain admins".into()]).is_ok());
        // AD account names may contain dots and spaces.
        assert!(validate_valid_users(&["CORP\\svc account.backup".into()]).is_ok());
    }

    #[test]
    fn validate_valid_users_rejects_injection_shapes() {
        // Same smuggling shapes validate_share_path pins (newline/config
        // injection), plus structural garbage.
        assert!(validate_valid_users(&["alice\ninclude = /etc/passwd".into()]).is_err());
        assert!(validate_valid_users(&["alice;rm".into()]).is_err());
        assert!(
            validate_valid_users(&["CORP\\\\alice".into()]).is_err(),
            "double backslash"
        );
        assert!(
            validate_valid_users(&["\\alice".into()]).is_err(),
            "empty domain part"
        );
        assert!(
            validate_valid_users(&["CORP\\".into()]).is_err(),
            "empty account part"
        );
        assert!(validate_valid_users(&["".into()]).is_err());
        assert!(validate_valid_users(&["THISNETBIOSNAMEISTOOLONG\\alice".into()]).is_err());
    }

    #[test]
    fn portal_share_policy_matches_direct_users_exactly_and_case_insensitively() {
        let mut share = minimal_share();
        share.valid_users = vec!["CORP\\Alice".into()];

        assert!(share_allows_principal(&share, "corp\\alice", &[]));
        assert!(!share_allows_principal(&share, "CORP\\alice2", &[]));
        assert!(!share_allows_principal(&share, "alice", &[]));
    }

    #[test]
    fn portal_share_policy_matches_local_and_domain_groups_with_spaces() {
        let mut share = minimal_share();
        share.valid_users = vec!["@staff".into(), "@CORP\\Domain Users".into()];

        assert!(share_allows_principal(&share, "alice", &["STAFF".into()]));
        assert!(share_allows_principal(
            &share,
            "CORP\\alice",
            &["corp\\domain users".into()]
        ));
        assert!(!share_allows_principal(
            &share,
            "CORP\\alice",
            &["CORP\\Domain User".into()]
        ));
    }

    #[test]
    fn portal_share_policy_denies_disabled_guest_and_unrestricted_shares() {
        let mut share = minimal_share();
        share.valid_users = vec!["alice".into()];

        share.enabled = false;
        assert!(!share_allows_principal(&share, "alice", &[]));
        share.enabled = true;
        share.guest_ok = true;
        assert!(!share_allows_principal(&share, "alice", &[]));
        share.guest_ok = false;
        share.valid_users.clear();
        assert!(!share_allows_principal(&share, "alice", &[]));
    }

    #[test]
    fn portal_share_policy_denies_all_raw_samba_parameters() {
        let mut share = minimal_share();
        share.valid_users = vec!["alice".into()];

        for key in ["valid users", "invalid users", "path", "read only"] {
            share.extra_params.clear();
            share.extra_params.insert(key.into(), "override".into());
            assert!(!share_allows_principal(&share, "alice", &[]));
        }
    }

    #[test]
    fn file_principal_validation_accepts_users_but_not_groups_or_whitespace() {
        assert!(validate_file_principal("alice").is_ok());
        assert!(validate_file_principal("CORP\\Alice Smith").is_ok());
        assert!(validate_file_principal("@staff").is_err());
        assert!(validate_file_principal(" alice").is_err());
        assert!(validate_file_principal("").is_err());
    }

    // ── render_share_conf / ksmbd.conf ──────────────────────────────

    #[test]
    fn render_share_conf_minimal() {
        let out = render_share_conf(&minimal_share());
        assert_eq!(
            out,
            "[share1]\n    \
             path = /fs/p/data\n    \
             read only = no\n    \
             browseable = yes\n    \
             guest ok = no\n"
        );
    }

    #[test]
    fn render_share_conf_with_comment() {
        let mut share = minimal_share();
        share.comment = Some("Family photos".to_string());
        let out = render_share_conf(&share);
        assert!(out.contains("    comment = Family photos\n"));
    }

    #[test]
    fn render_share_conf_guest_ok_emits_nobody_block() {
        let mut share = minimal_share();
        share.guest_ok = true;
        let out = render_share_conf(&share);
        assert!(out.contains("    guest ok = yes\n"));
        assert!(out.contains("    force user = nobody\n"));
        assert!(out.contains("    force group = nogroup\n"));
        assert!(out.contains("    create mask = 0666\n"));
        assert!(out.contains("    directory mask = 0777\n"));
    }

    #[test]
    fn render_share_conf_authenticated_force_user_picks_first_non_group() {
        let mut share = minimal_share();
        share.valid_users = vec![
            "@admins".to_string(),
            "alice".to_string(),
            "bob".to_string(),
        ];
        let out = render_share_conf(&share);
        assert!(out.contains("    force user = alice\n"));
        assert!(out.contains("    valid users = @admins alice bob\n"));
        assert!(out.contains("    create mask = 0664\n"));
        assert!(out.contains("    directory mask = 0775\n"));
    }

    #[test]
    fn render_share_conf_authenticated_all_groups_omits_force_user() {
        let mut share = minimal_share();
        share.valid_users = vec!["@admins".to_string(), "@users".to_string()];
        let out = render_share_conf(&share);
        assert!(!out.contains("force user"));
        assert!(out.contains("    valid users = @admins @users\n"));
    }

    #[test]
    fn render_share_conf_quotes_spaced_valid_users_entries() {
        let mut share = minimal_share();
        share.valid_users = vec!["@CORP\\domain admins".to_string(), "alice".to_string()];
        let conf = render_share_conf(&share);
        assert!(
            conf.contains("valid users = \"@CORP\\domain admins\" alice"),
            "{conf}"
        );
    }

    #[test]
    fn render_share_conf_extra_params_sorted_and_sanitized() {
        let mut share = minimal_share();
        share
            .extra_params
            .insert("zeta".to_string(), "1".to_string());
        share
            .extra_params
            .insert("alpha".to_string(), "two".to_string());
        share
            .extra_params
            .insert("middle".to_string(), "v;injected\nx".to_string());
        let out = render_share_conf(&share);
        let alpha_pos = out.find("alpha = two").unwrap();
        let middle_pos = out.find("middle = ").unwrap();
        let zeta_pos = out.find("zeta = 1").unwrap();
        assert!(alpha_pos < middle_pos && middle_pos < zeta_pos);
        assert!(out.contains("    middle = vinjectedx\n"));
    }

    #[test]
    fn render_ksmbd_conf_puts_global_then_enabled_shares() {
        let mut a = minimal_share();
        a.name = "beta".into();
        let mut b = minimal_share();
        b.name = "alpha".into();
        b.enabled = false;
        let mut c = minimal_share();
        c.name = "gamma".into();
        let out = render_ksmbd_conf(
            &[a, b, c],
            &SmbTuningGlobals {
                max_connections: 64,
                deadtime: 15,
            },
        );
        assert!(out.contains("[global]\n"));
        assert!(out.contains("    max connections = 64\n"));
        assert!(out.contains("    deadtime = 15\n"));
        assert!(out.contains("[beta]\n"));
        assert!(out.contains("[gamma]\n"));
        assert!(!out.contains("[alpha]\n"));
        let beta = out.find("[beta]").unwrap();
        let gamma = out.find("[gamma]").unwrap();
        assert!(beta < gamma, "shares sorted by name: {out}");
    }

    #[test]
    fn reject_time_machine_when_flag_set() {
        let mut share = minimal_share();
        assert!(reject_time_machine(&share).is_ok());
        share.time_machine = true;
        assert!(matches!(
            reject_time_machine(&share),
            Err(SmbError::TimeMachineUnsupported)
        ));
    }
}
