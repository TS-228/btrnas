//! Dynamic protocol management: enable/disable NFS, SMB, FTP, SFTP, S3, iSCSI, NVMe-oF at runtime.
//!
//! Persists state to `/var/lib/nasty/protocols.json` so boot-time services
//! know which protocols to start.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use tracing::{info, warn};

const STATE_PATH: &str = "/var/lib/nasty/protocols.json";
const KSMBD_CONF: &str = "/etc/ksmbd/ksmbd.conf";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Protocol {
    Nfs,
    Smb,
    Iscsi,
    Nvmeof,
    Ftp,
    Sftp,
    S3,
    Nut,
    Ssh,
    Avahi,
    Smart,
    RestServer,
}

impl Protocol {
    pub const ALL: &[Protocol] = &[
        Protocol::Nfs,
        Protocol::Smb,
        Protocol::Iscsi,
        Protocol::Nvmeof,
        Protocol::Ftp,
        Protocol::Sftp,
        Protocol::S3,
        Protocol::Nut,
        Protocol::Ssh,
        Protocol::Avahi,
        Protocol::Smart,
        Protocol::RestServer,
    ];

    pub fn is_system_service(&self) -> bool {
        matches!(
            self,
            Protocol::Nut
                | Protocol::Ssh
                | Protocol::Avahi
                | Protocol::Smart
                | Protocol::RestServer
        )
    }

    pub fn name(&self) -> &'static str {
        match self {
            Protocol::Nfs => "nfs",
            Protocol::Smb => "smb",
            Protocol::Iscsi => "iscsi",
            Protocol::Nvmeof => "nvmeof",
            Protocol::Ftp => "ftp",
            Protocol::Sftp => "sftp",
            Protocol::S3 => "s3",
            Protocol::Nut => "nut",
            Protocol::Ssh => "ssh",
            Protocol::Avahi => "avahi",
            Protocol::Smart => "smart",
            Protocol::RestServer => "rest-server",
        }
    }

    pub fn display_name(&self) -> &'static str {
        match self {
            Protocol::Nfs => "NFS",
            Protocol::Smb => "SMB",
            Protocol::Iscsi => "iSCSI",
            Protocol::Nvmeof => "NVMe-oF",
            Protocol::Ftp => "FTP",
            Protocol::Sftp => "SFTP",
            Protocol::S3 => "S3",
            Protocol::Nut => "UPS (NUT)",
            Protocol::Ssh => "SSH",
            Protocol::Avahi => "mDNS (Avahi)",
            Protocol::Smart => "SMART",
            Protocol::RestServer => "Backup Server",
        }
    }

    /// systemd service(s) to start/stop for this protocol
    fn services(&self) -> &[&str] {
        match self {
            Protocol::Nfs => &["nfs-server.service"],
            Protocol::Smb => &["ksmbd.service", "wsdd2.service"],
            Protocol::Iscsi => &["target.service"],
            Protocol::Nvmeof => &[], // configfs-based, no daemon
            Protocol::Ftp => &["nasty-rclone@ftp.service"],
            Protocol::Sftp => &["nasty-rclone@sftp.service"],
            Protocol::S3 => &["nasty-rclone@s3.service"],
            // NUT runs a different subset of services in local vs
            // remote modes — see `nut::services_for_mode`.  Remote
            // mode only needs upsmon; upsd/driver have nothing local
            // to talk to.  We read the mode synchronously here so
            // start/stop sequencing in this trait stays sync.
            Protocol::Nut => crate::nut::services_for_mode(crate::nut::mode_sync()),
            Protocol::Ssh => &["sshd.service"],
            Protocol::Avahi => &["avahi-daemon.service"],
            Protocol::Smart => &["smartd.service"],
            Protocol::RestServer => &["nasty-rest-server.service"],
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "nfs" => Some(Protocol::Nfs),
            "smb" => Some(Protocol::Smb),
            "iscsi" => Some(Protocol::Iscsi),
            "nvmeof" => Some(Protocol::Nvmeof),
            "ftp" => Some(Protocol::Ftp),
            "sftp" => Some(Protocol::Sftp),
            "s3" => Some(Protocol::S3),
            "nut" => Some(Protocol::Nut),
            "ssh" => Some(Protocol::Ssh),
            "avahi" => Some(Protocol::Avahi),
            "smart" => Some(Protocol::Smart),
            "rest-server" => Some(Protocol::RestServer),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ProtocolStatus {
    /// Machine-readable protocol identifier (e.g. `nfs`, `smb`, `iscsi`).
    pub name: String,
    /// Human-readable display name (e.g. `NFS`, `SMB`, `iSCSI`).
    pub display_name: String,
    /// Whether the protocol is enabled in persistent state.
    pub enabled: bool,
    /// Whether the protocol's systemd service is currently active.
    pub running: bool,
    /// Whether this is a system-level service (SSH, Avahi, SMART) rather than a storage protocol.
    pub system_service: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ProtocolState {
    #[serde(default)]
    nfs: bool,
    #[serde(default)]
    smb: bool,
    #[serde(default)]
    iscsi: bool,
    #[serde(default)]
    nvmeof: bool,
    #[serde(default)]
    ftp: bool,
    #[serde(default)]
    sftp: bool,
    #[serde(default)]
    s3: bool,
    #[serde(default)]
    nut: bool,
    #[serde(default = "default_true")]
    ssh: bool,
    #[serde(default = "default_true")]
    avahi: bool,
    #[serde(default = "default_true")]
    smart: bool,
    #[serde(default)]
    rest_server: bool,
}

fn default_true() -> bool {
    true
}

impl Default for ProtocolState {
    fn default() -> Self {
        Self {
            nfs: false,
            smb: false,
            iscsi: false,
            nvmeof: false,
            ftp: false,
            sftp: false,
            s3: false,
            nut: false,
            ssh: true,
            avahi: true,
            smart: true,
            rest_server: false,
        }
    }
}

impl ProtocolState {
    fn get(&self, proto: Protocol) -> bool {
        match proto {
            Protocol::Nfs => self.nfs,
            Protocol::Smb => self.smb,
            Protocol::Iscsi => self.iscsi,
            Protocol::Nvmeof => self.nvmeof,
            Protocol::Ftp => self.ftp,
            Protocol::Sftp => self.sftp,
            Protocol::S3 => self.s3,
            Protocol::Nut => self.nut,
            Protocol::Ssh => self.ssh,
            Protocol::Avahi => self.avahi,
            Protocol::Smart => self.smart,
            Protocol::RestServer => self.rest_server,
        }
    }

    fn set(&mut self, proto: Protocol, enabled: bool) {
        match proto {
            Protocol::Nfs => self.nfs = enabled,
            Protocol::Smb => self.smb = enabled,
            Protocol::Iscsi => self.iscsi = enabled,
            Protocol::Nvmeof => self.nvmeof = enabled,
            Protocol::Ftp => self.ftp = enabled,
            Protocol::Sftp => self.sftp = enabled,
            Protocol::S3 => self.s3 = enabled,
            Protocol::Nut => self.nut = enabled,
            Protocol::Ssh => self.ssh = enabled,
            Protocol::Avahi => self.avahi = enabled,
            Protocol::Smart => self.smart = enabled,
            Protocol::RestServer => self.rest_server = enabled,
        }
    }
}

pub struct ProtocolService;

impl Default for ProtocolService {
    fn default() -> Self {
        Self::new()
    }
}

impl ProtocolService {
    pub fn new() -> Self {
        Self
    }

    /// Restore enabled protocol services on startup.
    /// Starts daemons and loads kernel modules for protocols the user enabled.
    pub async fn restore(&self) {
        self.restore_excluding(&std::collections::HashSet::new())
            .await;
    }

    /// Restore enabled protocols except entries whose backing state failed
    /// safety validation during boot.
    pub async fn restore_excluding(&self, excluded: &std::collections::HashSet<Protocol>) {
        let state = load_state().await;

        for &proto in Protocol::ALL {
            let enabled = state.get(proto);
            if !enabled || excluded.contains(&proto) {
                if excluded.contains(&proto) {
                    warn!(
                        "Skipping {} restore because its persisted backing state is unsafe",
                        proto.display_name()
                    );
                }

                // smartd used to be pulled in directly by multi-user.target.
                // Stop an old-generation instance during a live upgrade when
                // the persisted protocol preference says SMART is disabled.
                if !enabled && proto == Protocol::Smart {
                    for svc in proto.services() {
                        if let Err(e) = systemctl("stop", svc).await {
                            warn!("Failed to stop disabled {svc}: {e}");
                        }
                    }
                }
                continue;
            }

            info!("Restoring protocol: {}", proto.display_name());

            prepare_protocol(proto).await;

            // Load kernel modules before starting services (iSCSI/NVMe-oF
            // services require the LIO/nvmet modules to already be present)
            for module in protocol_modules(proto).await {
                if let Err(e) = modprobe(module).await {
                    warn!("{e}");
                }
            }

            // Start associated services — auto-disable if any fail
            let mut failed = false;
            for svc in proto.services() {
                if let Err(e) = systemctl("start", svc).await {
                    warn!("Failed to start {svc}: {e}");
                    failed = true;
                    break;
                }
            }
            if failed {
                warn!(
                    "Auto-disabling {} — service units not available",
                    proto.display_name()
                );
                let mut state = load_state().await;
                state.set(proto, false);
                if let Err(e) = save_state(&state).await {
                    // The in-memory state shows disabled, but at next
                    // engine restart we'll re-evaluate from the
                    // persisted state — so the auto-disable will be
                    // forgotten and the user will see the protocol
                    // re-enabling itself.
                    warn!(
                        "auto-disable persistence for {} failed: {e}",
                        proto.display_name()
                    );
                }
            }

            // NFS started with the RDMA toggle on: add the rdma
            // listener. Warn-only — TCP NFS must keep working even if
            // the RDMA side can't come up (#602).
            if !failed
                && proto == Protocol::Nfs
                && crate::rdma::enabled().await
                && let Err(e) = crate::rdma::activate_nfs_rdma().await
            {
                warn!("NFS-over-RDMA activation failed (TCP NFS unaffected): {e}");
            }
        }
    }

    /// Stop a protocol's live services without changing the operator's
    /// persisted enabled preference. Used when boot-time safety validation
    /// fails and the protocol may have survived an engine-only restart.
    pub async fn quiesce(&self, proto: Protocol) -> Result<(), String> {
        for service in proto.services().iter().rev() {
            systemctl("stop", service).await?;
        }
        Ok(())
    }

    /// List all protocols with their enabled/running status
    pub async fn is_enabled(&self, proto: Protocol) -> bool {
        load_state().await.get(proto)
    }

    pub async fn is_running(&self, proto: Protocol) -> bool {
        is_protocol_running(proto).await
    }

    pub async fn list(&self) -> Vec<ProtocolStatus> {
        let state = load_state().await;
        let mut result = Vec::new();

        for &proto in Protocol::ALL {
            let running = is_protocol_running(proto).await;
            result.push(ProtocolStatus {
                name: proto.name().to_string(),
                display_name: proto.display_name().to_string(),
                enabled: state.get(proto),
                running,
                system_service: proto.is_system_service(),
            });
        }

        result
    }

    /// Enable a protocol: start its services and persist state
    pub async fn enable(&self, name: &str) -> Result<ProtocolStatus, String> {
        let proto = Protocol::from_name(name).ok_or_else(|| format!("unknown protocol: {name}"))?;

        let mut state = load_state().await;
        state.set(proto, true);
        save_state(&state).await?;

        prepare_protocol(proto).await;

        // Load kernel modules before starting services
        for module in protocol_modules(proto).await {
            if let Err(e) = modprobe(module).await {
                warn!("{e}");
            }
        }

        // Start associated services — roll back if any fail
        let services = proto.services();
        let mut started: Vec<&str> = Vec::new();
        for svc in services {
            info!(
                "Starting service {svc} for protocol {}",
                proto.display_name()
            );
            if let Err(e) = systemctl("start", svc).await {
                warn!("Failed to start {svc}: {e}");
                // Roll back: stop any services we already started.
                // systemctl() already logs spawn / non-zero failures
                // internally; this loop just makes sure we attempt
                // each one even if one of the stops fails.
                for started_svc in &started {
                    if let Err(stop_err) = systemctl("stop", started_svc).await {
                        warn!("rollback stop of {started_svc} failed: {stop_err}");
                    }
                }
                // Roll back persistent state. A failure here means
                // the protocol stays "enabled" in saved state but the
                // services aren't actually running — at next engine
                // start they'll be re-attempted, which is what we want,
                // but the operator should know the persistence flip
                // didn't take.
                state.set(proto, false);
                if let Err(save_err) = save_state(&state).await {
                    warn!(
                        "rollback persistence for {} failed: {save_err}",
                        proto.display_name()
                    );
                }
                return Err(format!("Failed to start {}: {e}", proto.display_name()));
            }
            started.push(*svc);
        }

        // Enabling SMB just started ksmbd; avahi may still advertise a
        // pre-SMB view until rebound onto the current network (#291).
        if proto == Protocol::Smb {
            crate::network::rebind_discovery_daemons().await;
        }

        // NFS started with the RDMA toggle on: add the rdma listener.
        // Warn-only — TCP NFS must keep working even if the RDMA side
        // can't come up (#602).
        if proto == Protocol::Nfs
            && crate::rdma::enabled().await
            && let Err(e) = crate::rdma::activate_nfs_rdma().await
        {
            warn!("NFS-over-RDMA activation failed (TCP NFS unaffected): {e}");
        }

        let running = is_protocol_running(proto).await;
        Ok(ProtocolStatus {
            name: proto.name().to_string(),
            display_name: proto.display_name().to_string(),
            enabled: true,
            running,
            system_service: proto.is_system_service(),
        })
    }

    /// Disable a protocol: stop its services and persist state
    pub async fn disable(&self, name: &str) -> Result<ProtocolStatus, String> {
        let proto = Protocol::from_name(name).ok_or_else(|| format!("unknown protocol: {name}"))?;

        let mut state = load_state().await;
        state.set(proto, false);
        save_state(&state).await?;

        // Stop associated services
        for svc in proto.services() {
            info!(
                "Stopping service {svc} for protocol {}",
                proto.display_name()
            );
            if let Err(e) = systemctl("stop", svc).await {
                warn!("Failed to stop {svc}: {e}");
            }
        }

        let running = is_protocol_running(proto).await;
        Ok(ProtocolStatus {
            name: proto.name().to_string(),
            display_name: proto.display_name().to_string(),
            enabled: false,
            running,
            system_service: proto.is_system_service(),
        })
    }
}

/// Check if a protocol is currently running
async fn is_protocol_running(proto: Protocol) -> bool {
    match proto {
        Protocol::Nfs => systemctl_is_active("nfs-server.service").await,
        Protocol::Smb => systemctl_is_active("ksmbd.service").await,
        Protocol::Iscsi => systemctl_is_active("target.service").await,
        Protocol::Nvmeof => {
            // NVMe-oF is "running" if nvmet configfs is available
            std::path::Path::new("/sys/kernel/config/nvmet").exists()
        }
        Protocol::Ftp => systemctl_is_active("nasty-rclone@ftp.service").await,
        Protocol::Sftp => systemctl_is_active("nasty-rclone@sftp.service").await,
        Protocol::S3 => systemctl_is_active("nasty-rclone@s3.service").await,
        Protocol::Nut => {
            // Pick the canary unit based on mode — upsd doesn't run
            // in remote mode, so checking nut-server there would
            // always report "down" even though monitoring is healthy.
            systemctl_is_active(crate::nut::status_unit(crate::nut::mode_sync())).await
        }
        Protocol::Ssh => systemctl_is_active("sshd.service").await,
        Protocol::Avahi => systemctl_is_active("avahi-daemon.service").await,
        Protocol::Smart => systemctl_is_active("smartd.service").await,
        Protocol::RestServer => systemctl_is_active("nasty-rest-server.service").await,
    }
}

/// Ensure prerequisites exist before starting a protocol's services.
async fn prepare_protocol(proto: Protocol) {
    if proto == Protocol::Smb {
        // ksmbd.mountd requires a parseable ksmbd.conf before start.
        if !std::path::Path::new(KSMBD_CONF).exists() {
            let header = "# Managed by NASty — do not edit manually\n\n[global]\n    workgroup = WORKGROUP\n    server string = NASty\n";
            if let Err(e) = tokio::fs::write(KSMBD_CONF, header).await {
                warn!("Failed to create {KSMBD_CONF}: {e}");
            }
        }
    }
    if proto == Protocol::Nut {
        // Write NUT config files from persisted config before starting daemons
        let config = crate::nut::load_config().await;
        if let Err(e) = crate::nut::write_config_files(&config).await {
            warn!("Failed to write NUT config files: {e}");
        }
    }
    if proto == Protocol::RestServer {
        // The rest-server systemd unit requires an htpasswd file
        // (no more --no-auth). Generate the credentials before
        // starting the service so the start path never sees a
        // missing file. Idempotent — does nothing when both the
        // state file and htpasswd are already in place.
        if let Err(e) = crate::rest_server::ensure_credentials().await {
            warn!(
                "Failed to ensure rest-server credentials: {e}. \
                 The service will fail to start until this is resolved."
            );
        }
    }
}

/// Kernel modules a protocol needs before its services start. The
/// RDMA additions are gated on the per-box toggle so an unopted box
/// never loads transport modules it can't use (#602).
async fn protocol_modules(proto: Protocol) -> Vec<&'static str> {
    let rdma = crate::rdma::enabled().await;
    match proto {
        Protocol::Iscsi => {
            let mut m = vec!["target_core_mod", "iscsi_target_mod"];
            if rdma {
                m.push("ib_isert");
            }
            m
        }
        Protocol::Nvmeof => {
            let mut m = vec!["nvmet", "nvmet-tcp"];
            if rdma {
                m.push("nvmet-rdma");
            }
            m
        }
        _ => vec![],
    }
}

pub(crate) async fn systemctl(action: &str, service: &str) -> Result<(), String> {
    let mut command = tokio::process::Command::new("systemctl");
    command.args([action, service]).kill_on_drop(true);
    let output = tokio::time::timeout(std::time::Duration::from_secs(30), command.output())
        .await
        .map_err(|_| format!("systemctl {action} {service} timed out after 30 seconds"))?
        .map_err(|e| format!("failed to run systemctl: {e}"))?;

    if output.status.success() {
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        Err(format!("systemctl {action} {service} failed: {stderr}"))
    }
}

pub(crate) async fn systemctl_is_active(service: &str) -> bool {
    tokio::process::Command::new("systemctl")
        .args(["is-active", "--quiet", service])
        .status()
        .await
        .map(|s| s.success())
        .unwrap_or(false)
}

pub(crate) async fn modprobe(module: &str) -> Result<(), String> {
    let output = tokio::process::Command::new("modprobe")
        .arg(module)
        .output()
        .await
        .map_err(|e| format!("modprobe {module} failed: {e}"))?;

    if output.status.success() {
        Ok(())
    } else {
        Err(format!("modprobe {module} failed"))
    }
}

async fn is_virtual_machine() -> bool {
    tokio::process::Command::new("systemd-detect-virt")
        .arg("--vm")
        .status()
        .await
        .map(|s| s.success())
        .unwrap_or(false)
}

async fn load_state() -> ProtocolState {
    match tokio::fs::read_to_string(STATE_PATH).await {
        Ok(content) => match serde_json::from_str(&content) {
            Ok(state) => state,
            Err(e) => {
                warn!("Failed to parse protocol state, resetting to defaults: {e}");
                ProtocolState::default()
            }
        },
        Err(_) => {
            // Fresh install: disable SMART by default on VMs because virtual
            // disks usually do not expose it. Operators with passed-through
            // disks or controllers can explicitly enable monitoring.
            // Persist immediately — this function is called on every
            // protocol-status refresh, so without persistence the file
            // never appears, and the VM detection (plus its info!) runs
            // forever every minute.
            let mut state = ProtocolState::default();
            if is_virtual_machine().await {
                info!("Virtual machine detected — disabling SMART by default");
                state.smart = false;
            }
            if let Err(e) = save_state(&state).await {
                warn!("Failed to persist initial protocol state: {e}");
            }
            state
        }
    }
}

async fn save_state(state: &ProtocolState) -> Result<(), String> {
    let json = serde_json::to_string_pretty(state)
        .map_err(|e| format!("failed to serialize protocol state: {e}"))?;
    tokio::fs::write(STATE_PATH, json)
        .await
        .map_err(|e| format!("failed to write protocol state: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn smb_protocol_uses_ksmbd_service() {
        let svcs = Protocol::Smb.services();
        assert_eq!(svcs, &["ksmbd.service", "wsdd2.service"]);
    }

    #[test]
    fn rclone_protocols_use_templated_units() {
        assert_eq!(Protocol::Ftp.services(), &["nasty-rclone@ftp.service"]);
        assert_eq!(Protocol::Sftp.services(), &["nasty-rclone@sftp.service"]);
        assert_eq!(Protocol::S3.services(), &["nasty-rclone@s3.service"]);
        assert_eq!(Protocol::from_name("ftp"), Some(Protocol::Ftp));
        assert_eq!(Protocol::from_name("sftp"), Some(Protocol::Sftp));
        assert_eq!(Protocol::from_name("s3"), Some(Protocol::S3));
    }
}
