//! Aggregate "what depends on this filesystem" across services.
//!
//! Backs the impact-preview dialog before destructive FS operations
//! (currently the encrypted-FS Lock action — see issue #86's discussion).
//! Walks each downstream service that can reference a filesystem path,
//! returns names/IDs grouped by service. Read-only — never mutates.
//!
//! Matching strategy: most services store a path. We treat a path as
//! "depending on FS X" when it lives under `/fs/<X>/` (the canonical
//! NASty mount root, see `NASTY_MOUNT_BASE` in nasty-storage). Paths
//! that point to `/dev/loopN` are mapped to the underlying subvolume's
//! filesystem via `Subvolume.block_device` — that's how VM disks and
//! iSCSI/NVMe-oF backstores get detected.
//!
//! This module deliberately doesn't `tokio::join!` the queries —
//! list-call latency is dominated by `apps.list()` (Docker round-trip)
//! and parallelizing the others doesn't move the wall clock noticeably.
//! Sequential keeps the failure mode trivial: one slow service blocks
//! the rest, but the dialog is acceptable up to several hundred ms.

use std::collections::HashSet;

use schemars::JsonSchema;
use serde::Serialize;

use crate::AppState;

/// Names/IDs of every downstream entity that depends on a given
/// filesystem. Empty fields are serialized as `[]` so the WebUI can
/// render unconditionally without null-checking.
#[derive(Debug, Clone, Default, Serialize, JsonSchema)]
pub struct FsDependents {
    pub filesystem: String,
    pub mounted: bool,
    pub subvolumes: Vec<String>,
    pub apps_storage: bool,
    pub apps: Vec<String>,
    pub vms: Vec<String>,
    pub backup_jobs: Vec<String>,
    pub nfs_shares: Vec<String>,
    pub smb_shares: Vec<String>,
    pub ftp_shares: Vec<String>,
    pub sftp_shares: Vec<String>,
    pub s3_shares: Vec<String>,
    pub iscsi_targets: Vec<String>,
    pub nvmeof_subsystems: Vec<String>,
    pub state_errors: Vec<String>,
}

impl FsDependents {
    pub fn has_dependents(&self) -> bool {
        !self.subvolumes.is_empty()
            || self.apps_storage
            || !self.apps.is_empty()
            || !self.vms.is_empty()
            || !self.backup_jobs.is_empty()
            || !self.nfs_shares.is_empty()
            || !self.smb_shares.is_empty()
            || !self.ftp_shares.is_empty()
            || !self.sftp_shares.is_empty()
            || !self.s3_shares.is_empty()
            || !self.iscsi_targets.is_empty()
            || !self.nvmeof_subsystems.is_empty()
    }
}

/// True if `path` falls under `/fs/<fs_name>/` (the canonical NASty
/// mount root). Trailing slash matters: without it `/fs/tank2` would
/// match a query for FS `tank`.
fn path_belongs_to_fs(path: &str, fs_name: &str) -> bool {
    let prefix = format!("/fs/{fs_name}/");
    path.starts_with(&prefix) || path == format!("/fs/{fs_name}")
}

/// Build the dependents view by querying every service that can hold
/// a filesystem reference. State read failures are reported separately so
/// destructive callers can fail closed.
pub async fn find_dependents(state: &AppState, fs_name: &str) -> FsDependents {
    find_dependents_with_uuid(state, fs_name, None).await
}

/// UUID-aware variant used when the filesystem itself is unavailable. Stable
/// block-export identities remain inspectable even when no subvolume can be
/// discovered from the missing mount.
pub async fn find_dependents_with_uuid(
    state: &AppState,
    fs_name: &str,
    filesystem_uuid: Option<&str>,
) -> FsDependents {
    let mut out = FsDependents {
        filesystem: fs_name.to_string(),
        ..Default::default()
    };

    // Filesystem mount state — orients the UI message ("currently
    // mounted, will be unmounted" vs "already unmounted, only
    // revoking the key").
    if let Ok(fs) = state.filesystems.get(fs_name).await {
        out.mounted = fs.mounted;
    }

    // Subvolumes are the cheapest hop: we already filter by fs.
    let subvols = match state.subvolumes.list_all(Some(fs_name), None).await {
        Ok(subvolumes) => subvolumes,
        Err(error) => {
            out.state_errors
                .push(format!("subvolume state failed to load: {error}"));
            Vec::new()
        }
    };
    // Block devices owned by subvolumes on this FS — used to detect
    // VM disks / iSCSI backstores / NVMe-oF namespaces that reference
    // them by `/dev/loopN` rather than path.
    let block_devs: HashSet<String> = subvols
        .iter()
        .filter_map(|s| s.block_device.clone())
        .collect();
    let block_ids: HashSet<nasty_common::BlockVolumeId> = subvols
        .iter()
        .filter_map(|s| s.block_volume_id.clone())
        .collect();
    out.subvolumes = subvols.into_iter().map(|s| s.name).collect();

    // Apps: when the docker storage is on this FS, every app is on
    // it (their layered images, named volumes, default bind base
    // all live under the apps storage path). Per-app bind mounts
    // outside that base are a refinement we can add later — they're
    // surfaced via app inspect, not the lightweight `list()`.
    match nasty_apps::AppsService::load_config_strict() {
        Ok(config)
            if [
                config.storage_path.as_deref(),
                config.appdata_path.as_deref(),
            ]
            .into_iter()
            .flatten()
            .any(|path| path_belongs_to_fs(path, fs_name)) =>
        {
            out.apps_storage = true;
            match state.apps.list().await {
                Ok(apps) => out.apps = apps.into_iter().map(|app| app.name).collect(),
                Err(error) => out
                    .state_errors
                    .push(format!("app state failed to load: {error}")),
            }
        }
        Ok(_) => {}
        Err(error) => out
            .state_errors
            .push(format!("app config failed to load: {error}")),
    }

    // VMs: disk path either under /fs/<X>/... directly, or a loop
    // device that's the block_device of a subvolume on this FS.
    match state.vms.list().await {
        Ok(vms) => {
            for vm in vms {
                let touches_fs = vm.config.disks.iter().any(|disk| {
                    path_belongs_to_fs(&disk.path, fs_name)
                        || disk
                            .source
                            .as_deref()
                            .is_some_and(|source| path_belongs_to_fs(source, fs_name))
                        || block_devs.contains(&disk.path)
                });
                if touches_fs {
                    out.vms.push(vm.config.name);
                }
            }
        }
        Err(error) => out
            .state_errors
            .push(format!("VM state failed to load: {error}")),
    }

    // Backup jobs: any source under /fs/<X>/. A job pointing only
    // somewhere else (e.g. a subset of a different FS) is left alone.
    match state.backups.list_profiles_strict().await {
        Ok(profiles) => {
            for profile in profiles {
                if profile
                    .sources
                    .iter()
                    .any(|source| path_belongs_to_fs(source, fs_name))
                {
                    out.backup_jobs.push(profile.name);
                }
            }
        }
        Err(error) => out
            .state_errors
            .push(format!("backup state failed to load: {error}")),
    }

    // Shares: NFS/SMB use a single `path`; iSCSI/NVMe-oF use device
    // paths that are usually loop devices for block subvolumes.
    match state.nfs.list_strict().await {
        Ok(shares) => {
            out.nfs_shares = shares
                .into_iter()
                .filter(|s| path_belongs_to_fs(&s.path, fs_name))
                .map(|s| s.id)
                .collect();
        }
        Err(error) => out
            .state_errors
            .push(format!("NFS state failed to load: {error}")),
    }
    match state.smb.list_strict().await {
        Ok(shares) => {
            out.smb_shares = shares
                .into_iter()
                .filter(|s| path_belongs_to_fs(&s.path, fs_name))
                .map(|s| s.name)
                .collect();
        }
        Err(error) => out
            .state_errors
            .push(format!("SMB state failed to load: {error}")),
    }
    match state.ftp.list_strict().await {
        Ok(shares) => {
            out.ftp_shares = shares
                .into_iter()
                .filter(|s| path_belongs_to_fs(&s.path, fs_name))
                .map(|s| s.name)
                .collect();
        }
        Err(error) => out
            .state_errors
            .push(format!("FTP state failed to load: {error}")),
    }
    match state.sftp.list_strict().await {
        Ok(shares) => {
            out.sftp_shares = shares
                .into_iter()
                .filter(|s| path_belongs_to_fs(&s.path, fs_name))
                .map(|s| s.name)
                .collect();
        }
        Err(error) => out
            .state_errors
            .push(format!("SFTP state failed to load: {error}")),
    }
    match state.s3.list_strict().await {
        Ok(shares) => {
            out.s3_shares = shares
                .into_iter()
                .filter(|s| path_belongs_to_fs(&s.path, fs_name))
                .map(|s| s.name)
                .collect();
        }
        Err(error) => out
            .state_errors
            .push(format!("S3 state failed to load: {error}")),
    }
    match state.iscsi.list().await {
        Ok(targets) => {
            for target in &targets {
                if target
                    .luns
                    .iter()
                    .any(|lun| lun.backing_volume_unresolved && lun.backing_volume.is_none())
                {
                    out.state_errors.push(format!(
                        "iSCSI target '{}' has unresolved legacy block-volume ownership",
                        target.iqn
                    ));
                }
            }
            out.iscsi_targets = targets
                .into_iter()
                .filter(|t| {
                    t.luns.iter().any(|l| {
                        path_belongs_to_fs(&l.backstore_path, fs_name)
                            || block_devs.contains(&l.backstore_path)
                            || l.backing_volume.as_ref().is_some_and(|identity| {
                                block_ids.contains(identity)
                                    || filesystem_uuid == Some(identity.filesystem_uuid.as_str())
                            })
                    })
                })
                .map(|t| t.iqn)
                .collect();
        }
        Err(error) => out
            .state_errors
            .push(format!("iSCSI state failed to load: {error}")),
    }
    match state.nvmeof.list().await {
        Ok(subs) => {
            for subsystem in &subs {
                if subsystem.namespaces.iter().any(|namespace| {
                    namespace.backing_volume_unresolved && namespace.backing_volume.is_none()
                }) {
                    out.state_errors.push(format!(
                        "NVMe-oF subsystem '{}' has unresolved legacy block-volume ownership",
                        subsystem.nqn
                    ));
                }
            }
            out.nvmeof_subsystems = subs
                .into_iter()
                .filter(|s| {
                    s.namespaces.iter().any(|n| {
                        path_belongs_to_fs(&n.device_path, fs_name)
                            || block_devs.contains(&n.device_path)
                            || n.backing_volume.as_ref().is_some_and(|identity| {
                                block_ids.contains(identity)
                                    || filesystem_uuid == Some(identity.filesystem_uuid.as_str())
                            })
                    })
                })
                .map(|s| s.nqn)
                .collect();
        }
        Err(error) => out
            .state_errors
            .push(format!("NVMe-oF state failed to load: {error}")),
    }

    out
}

/// Reverse-index of currently-locked encrypted filesystems → what
/// would come back to life if they were unlocked. Powers the
/// "🔒 on tank" badges on the Apps and VMs pages: those pages need
/// to know "is *my* app/VM blocked by a locked FS, and which one?"
/// without hitting `find_dependents` per FS in the browser.
///
/// Only includes FSes that are currently encrypted AND locked AND
/// have at least one app or VM among their dependents — empty
/// entries would just be wire bytes the UI filters back out.
pub async fn find_locked_dependents(state: &AppState) -> Vec<FsDependents> {
    let Ok(filesystems) = state.filesystems.list().await else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for fs in filesystems {
        // Only encrypted-and-locked is interesting here. A plain
        // unmounted FS has no badge story; an encrypted-but-unlocked
        // one is just waiting for `fs.mount`.
        if fs.options.encrypted != Some(true) || fs.options.locked != Some(true) {
            continue;
        }
        let deps = find_dependents(state, &fs.name).await;
        if !deps.apps.is_empty() || !deps.vms.is_empty() {
            out.push(deps);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn path_match_handles_prefix_and_exact() {
        // Trailing slash matters — without it /fs/tank2 would falsely
        // match a query for /fs/tank.
        assert!(path_belongs_to_fs("/fs/tank/foo", "tank"));
        assert!(path_belongs_to_fs("/fs/tank/sub/file", "tank"));
        // Exact match (no trailing slash, no children) — the FS
        // mount-point itself, not a child path.
        assert!(path_belongs_to_fs("/fs/tank", "tank"));
        // Sibling-prefix attack: /fs/tank2 must not match `tank`.
        assert!(!path_belongs_to_fs("/fs/tank2/foo", "tank"));
        assert!(!path_belongs_to_fs("/fs/tank2", "tank"));
        // Other roots untouched.
        assert!(!path_belongs_to_fs("/var/lib/nasty", "tank"));
        assert!(!path_belongs_to_fs("", "tank"));
    }

    #[test]
    fn dependency_presence_is_separate_from_state_errors() {
        let mut dependents = FsDependents {
            state_errors: vec!["unreadable".to_string()],
            ..Default::default()
        };
        assert!(!dependents.has_dependents());

        dependents.nvmeof_subsystems.push("nqn.test".to_string());
        assert!(dependents.has_dependents());
    }
}
