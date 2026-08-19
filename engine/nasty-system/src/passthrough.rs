//! Persistent vfio-pci passthrough configuration.
//!
//! Users mark individual PCI devices (by BDF address) to be claimed by
//! `vfio-pci` at boot, before regular drivers can grab them. The flow:
//!
//! ```text
//! WebUI toggle (per device)
//!   ↓
//! state file: /var/lib/nasty/passthrough.json
//!   ↓
//! engine writes: /etc/nixos/passthrough.nix
//!   `services.udev.extraRules` — one rule per device setting
//!   `driver_override=vfio-pci` at PCI add time (the driverctl
//!   approach), so only vfio-pci may ever bind that device
//!   ↓
//! wrapper flake imports passthrough.nix
//!   ↓
//! next nixos-rebuild + reboot — vfio-pci owns the device from boot
//! ```
//!
//! Claims are keyed by **BDF address**, not vendor:device pairs. The
//! earlier pair-based `vfio-pci.ids=` mechanism couldn't tell identical
//! devices apart — claiming one SR-IOV VF claimed *every* VF of the
//! card (they share one device ID), including VFs meant for host
//! networking, and the management-interface guard couldn't distinguish
//! the mgmt VF from a sibling. Per-BDF `driver_override` udev rules
//! give exact one-device granularity and match the runtime bind path
//! (`bind_vfio` in nasty-vm, #603/#601).
//!
//! Trade-off: a BDF changes if the card moves slots (pairs survived
//! that). A claim whose device vanished is harmless — the udev rule
//! matches nothing — and the WebUI shows claims with their recorded
//! vendor/device names so the user can re-claim after a hardware move.
//!
//! ## Legacy state compatibility (engine self-update invariant)
//!
//! Pre-BDF versions persisted `{ ids: [{vendor, device}] }`. Both
//! directions must keep working across an engine rollback:
//!
//! - **Migration in**: when `devices` is empty but `ids` isn't, each
//!   pair is resolved against the live PCI bus to *all* matching BDFs
//!   (preserving the old claim-every-match semantics) on first save.
//! - **Mirror out**: every save also writes `ids` derived from
//!   `devices`, so an older engine rolled back onto this state file
//!   still renders a functional (coarser) `vfio-pci.ids=` config.
//! - **Divergence**: if an old engine edited `ids` after a rollback,
//!   the pair set no longer matches `devices`; the newer engine then
//!   treats `ids` as the fresher intent and re-resolves it.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use tokio::sync::Mutex;
use tracing::{info, warn};

const STATE_PATH: &str = "/var/lib/nasty/passthrough.json";
/// Debian: write vfio udev rules (was a NixOS module overlay).
const UDEV_RULES_PATH: &str = "/etc/udev/rules.d/99-nasty-passthrough.rules";
const STATE_MARKER_PATH: &str = "/etc/nasty/passthrough.udev";

/// One PCI vendor:device identifier. Retained for the legacy mirror
/// (`PassthroughConfig::ids`) and for display.
#[derive(
    Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema, Hash,
)]
pub struct DeviceId {
    /// 4-hex-digit vendor ID, e.g. `10de` (NVIDIA). Lowercase.
    pub vendor: String,
    /// 4-hex-digit device ID, e.g. `2204` (RTX 3080). Lowercase.
    pub device: String,
}

impl DeviceId {
    /// Render as the `vendor:device` form vfio-pci.ids consumes.
    pub fn to_vfio_id(&self) -> String {
        format!("{}:{}", self.vendor, self.device)
    }
}

/// One claimed PCI device — the granularity of the boot-time claim.
#[derive(
    Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema, Hash,
)]
pub struct PassthroughEntry {
    /// Full BDF address, e.g. `0000:06:00.1`.
    pub address: String,
    /// Vendor ID recorded at claim time — display + legacy mirror.
    pub vendor: String,
    /// Device ID recorded at claim time — display + legacy mirror.
    pub device: String,
}

/// Persistent passthrough config.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct PassthroughConfig {
    /// Per-device claims (authoritative).
    #[serde(default)]
    pub devices: Vec<PassthroughEntry>,
    /// Legacy vendor:device mirror, derived from `devices` on every
    /// save. Read by pre-BDF engines after a rollback; read by this
    /// engine only to migrate old state or absorb rollback-era edits.
    #[serde(default)]
    pub ids: Vec<DeviceId>,
}

#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
pub struct PassthroughUpdate {
    /// BDF addresses to claim. The engine records each device's
    /// vendor:device from sysfs at save time.
    #[serde(default)]
    pub addresses: Vec<String>,
    /// Legacy pair form — accepted for older clients; resolved to all
    /// matching live BDFs (the old semantics). Ignored when
    /// `addresses` is non-empty.
    #[serde(default)]
    pub ids: Vec<DeviceId>,
}

/// Lock held only across reads/writes so concurrent RPCs can't tear
/// the state file. Initialized lazily — no global setup needed.
static STATE_LOCK: Mutex<()> = Mutex::const_new(());

pub async fn load() -> PassthroughConfig {
    let _guard = STATE_LOCK.lock().await;
    let cfg: PassthroughConfig = nasty_common::load_singleton_or_recover(STATE_PATH).await;
    reconcile_legacy(cfg, &scan_pci_inventory().await)
}

/// Bring a loaded config to the per-device view, absorbing legacy
/// state. Pure — the PCI inventory is passed in for testability.
fn reconcile_legacy(
    mut cfg: PassthroughConfig,
    inventory: &[PciInventoryEntry],
) -> PassthroughConfig {
    let mirror = derive_ids(&cfg.devices);
    if cfg.devices.is_empty() && !cfg.ids.is_empty() {
        // Pre-BDF state file: resolve pairs to every matching device.
        cfg.devices = resolve_pairs(&cfg.ids, inventory);
        info!(
            "passthrough: migrated {} legacy id pair(s) to {} device claim(s)",
            cfg.ids.len(),
            cfg.devices.len()
        );
    } else if !cfg.devices.is_empty() && mirror != cfg.ids {
        // An older engine edited `ids` after a rollback — its edit is
        // the fresher intent; re-resolve and drop the stale devices.
        warn!(
            "passthrough: legacy ids diverge from device claims (rollback-era edit?) — \
             re-resolving from ids"
        );
        cfg.devices = resolve_pairs(&cfg.ids, inventory);
    }
    cfg
}

/// Apply a request: resolve to per-device claims, validate against the
/// management interface collision, persist to disk, and regenerate
/// `/etc/nixos/passthrough.nix`. Returns the saved config.
///
/// The change is **not active until reboot**. Users see a "Reboot
/// required" banner; the engine doesn't trigger nixos-rebuild
/// automatically (mirrors how PR #113 handled hostname.nix).
pub async fn save_and_apply(
    update: PassthroughUpdate,
    mgmt_iface: Option<&str>,
) -> Result<PassthroughConfig, String> {
    let inventory = scan_pci_inventory().await;

    let devices = if !update.addresses.is_empty() {
        let mut out = BTreeSet::new();
        for addr in update.addresses {
            let addr = addr.trim().to_ascii_lowercase();
            if !is_bdf(&addr) {
                return Err(format!(
                    "'{addr}' is not a PCI address (expected dddd:bb:dd.f, e.g. 0000:06:00.1)"
                ));
            }
            let Some(inv) = inventory.iter().find(|e| e.address == addr) else {
                return Err(format!(
                    "no PCI device at {addr} — it may have moved slots; refresh the \
                     hardware page and re-select it"
                ));
            };
            out.insert(PassthroughEntry {
                address: addr,
                vendor: inv.vendor.clone(),
                device: inv.device.clone(),
            });
        }
        out.into_iter().collect()
    } else {
        // Legacy pair request (old WebUI) — old claim-all semantics.
        resolve_pairs(&normalize_ids(update.ids), &inventory)
    };

    validate_request(&devices, mgmt_iface).await?;
    let cfg = PassthroughConfig {
        ids: derive_ids(&devices),
        devices,
    };

    let _guard = STATE_LOCK.lock().await;
    let json = serde_json::to_string_pretty(&cfg).map_err(|e| format!("serialize: {e}"))?;
    if let Some(parent) = std::path::Path::new(STATE_PATH).parent()
        && let Err(e) = tokio::fs::create_dir_all(parent).await
    {
        warn!("create {}: {e}", parent.display());
    }
    tokio::fs::write(STATE_PATH, json)
        .await
        .map_err(|e| format!("write {STATE_PATH}: {e}"))?;
    write_udev_rules(&cfg).await?;
    info!(
        "Passthrough config updated: {} device(s); reboot required to apply",
        cfg.devices.len()
    );
    Ok(cfg)
}

/// The legacy mirror: sorted, deduped pairs of the claimed devices.
fn derive_ids(devices: &[PassthroughEntry]) -> Vec<DeviceId> {
    devices
        .iter()
        .map(|d| DeviceId {
            vendor: d.vendor.clone(),
            device: d.device.clone(),
        })
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

/// Resolve pairs to every live device matching them — the semantics
/// the pair-based mechanism had.
fn resolve_pairs(ids: &[DeviceId], inventory: &[PciInventoryEntry]) -> Vec<PassthroughEntry> {
    let wanted: BTreeSet<(&str, &str)> = ids
        .iter()
        .map(|i| (i.vendor.as_str(), i.device.as_str()))
        .collect();
    inventory
        .iter()
        .filter(|e| wanted.contains(&(e.vendor.as_str(), e.device.as_str())))
        .map(|e| PassthroughEntry {
            address: e.address.clone(),
            vendor: e.vendor.clone(),
            device: e.device.clone(),
        })
        .collect()
}

/// Minimal live-PCI view for claim resolution.
struct PciInventoryEntry {
    address: String,
    vendor: String,
    device: String,
}

async fn scan_pci_inventory() -> Vec<PciInventoryEntry> {
    let mut out = Vec::new();
    let Ok(mut dir) = tokio::fs::read_dir("/sys/bus/pci/devices").await else {
        return out;
    };
    while let Ok(Some(entry)) = dir.next_entry().await {
        let address = entry.file_name().to_string_lossy().to_string();
        let path = entry.path();
        let (Some(vendor), Some(device)) = (
            read_pci_id_field(&path, "vendor").await,
            read_pci_id_field(&path, "device").await,
        ) else {
            continue;
        };
        out.push(PciInventoryEntry {
            address,
            vendor,
            device,
        });
    }
    out
}

/// Write vfio-pci udev rules for claimed devices. Empty config removes the rules file.
async fn write_udev_rules(cfg: &PassthroughConfig) -> Result<(), String> {
    for path in [UDEV_RULES_PATH, STATE_MARKER_PATH] {
        if let Some(parent) = std::path::Path::new(path).parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|e| format!("create {}: {e}", parent.display()))?;
        }
    }

    let body = render_udev_rules(cfg);
    if cfg.devices.is_empty() {
        let _ = tokio::fs::remove_file(UDEV_RULES_PATH).await;
        let _ = tokio::fs::remove_file(STATE_MARKER_PATH).await;
        return Ok(());
    }
    tokio::fs::write(UDEV_RULES_PATH, &body)
        .await
        .map_err(|e| format!("write {UDEV_RULES_PATH}: {e}"))?;
    // Marker under /etc/nasty for operators / recovery backups.
    tokio::fs::write(STATE_MARKER_PATH, &body)
        .await
        .map_err(|e| format!("write {STATE_MARKER_PATH}: {e}"))?;
    Ok(())
}

/// Pure renderer for `/etc/udev/rules.d/99-nasty-passthrough.rules`.
fn render_udev_rules(cfg: &PassthroughConfig) -> String {
    if cfg.devices.is_empty() {
        return String::new();
    }
    let mut out = String::from(
        "# Generated by nasty-engine. Reboot (or udevadm trigger) to apply.\n\
         # Source of truth: /var/lib/nasty/passthrough.json\n",
    );
    for d in &cfg.devices {
        out.push_str(&format!(
            "# {}:{}\nACTION==\"add\", SUBSYSTEM==\"pci\", KERNEL==\"{}\", \
             ATTR{{driver_override}}=\"vfio-pci\", RUN+=\"/bin/sh -c 'echo %k > \
             /sys/bus/pci/devices/%k/driver/unbind 2>/dev/null || true; echo %k > \
             /sys/bus/pci/drivers_probe'\"\n",
            d.vendor, d.device, d.address
        ));
    }
    out
}

/// Sort + dedupe; lowercase the hex strings. Skips obviously-malformed
/// entries (non-4-hex-digit ids).
fn normalize_ids(ids: Vec<DeviceId>) -> Vec<DeviceId> {
    let mut set = BTreeSet::new();
    for id in ids {
        let v = id.vendor.to_ascii_lowercase();
        let d = id.device.to_ascii_lowercase();
        if !is_hex4(&v) || !is_hex4(&d) {
            warn!(
                "ignoring malformed passthrough id: {}:{}",
                id.vendor, id.device
            );
            continue;
        }
        set.insert(DeviceId {
            vendor: v,
            device: d,
        });
    }
    set.into_iter().collect()
}

fn is_hex4(s: &str) -> bool {
    s.len() == 4 && s.chars().all(|c| c.is_ascii_hexdigit())
}

/// `dddd:bb:dd.f` — full BDF with domain, lowercase hex.
fn is_bdf(s: &str) -> bool {
    let bytes = s.as_bytes();
    if bytes.len() != 12 {
        return false;
    }
    s.char_indices().all(|(i, c)| match i {
        4 | 7 => c == ':',
        10 => c == '.',
        _ => c.is_ascii_hexdigit(),
    })
}

/// Refuse requests that would claim the device on which the caller is
/// reaching the engine. Returns `Ok(())` if safe, `Err(reason)` if the
/// request would brick connectivity at next boot.
///
/// Per-BDF now: the mgmt interface's exact device is refused, while an
/// identical sibling (same vendor:device, different address — e.g.
/// another SR-IOV VF of the same card) is allowed. That sibling case
/// was the pair-based check's blind spot in both directions.
///
/// Out of scope here (planned follow-ups):
/// - Boot disk's storage controller (refuse claiming it).
/// - IOMMU group-wide enforcement (some users do partial-group
///   passthrough with ACS overrides — refusing it outright would
///   block valid setups).
async fn validate_request(
    devices: &[PassthroughEntry],
    mgmt_iface: Option<&str>,
) -> Result<(), String> {
    let Some(iface) = mgmt_iface else {
        // Caller couldn't resolve the management iface — fail safe by
        // letting the user proceed. The classifier in the network
        // module makes the same conservative choice.
        return Ok(());
    };
    let Some(mgmt_bdf) = mgmt_iface_bdf(iface).await else {
        warn!(
            "passthrough: management iface '{iface}' has no resolvable PCI device — \
             skipping mgmt-collision check, user is on their own"
        );
        return Ok(());
    };
    if let Some(hit) = devices.iter().find(|d| d.address == mgmt_bdf) {
        return Err(format!(
            "refusing to claim {} ({}:{}) for passthrough — that is the PCI device \
             behind the management interface '{}', so vfio-pci would steal it at boot \
             and lock you out. Reach the box on a different interface (or console) \
             and retry.",
            hit.address, hit.vendor, hit.device, iface
        ));
    }
    Ok(())
}

/// Resolve `<iface>` → BDF by reading the `/sys/class/net/<iface>/device`
/// symlink.
async fn mgmt_iface_bdf(iface: &str) -> Option<String> {
    let device_link = format!("/sys/class/net/{iface}/device");
    let canonical = tokio::fs::canonicalize(&device_link).await.ok()?;
    bdf_from_device_path(&canonical)
}

/// Nearest ancestor directory whose name is a BDF. The `device`
/// symlink doesn't always land on the PCI function itself: virtio
/// NICs resolve to the virtio-bus child (`.../0000:07:12.0/virtio3`),
/// and the pre-BDF pair resolver silently mis-read that child's
/// virtio-bus vendor/device — which never matched a PCI pair, so the
/// mgmt guard was a no-op for virtio NICs. Walking up to the BDF
/// component fixes the guard for nested-bus devices.
fn bdf_from_device_path(path: &std::path::Path) -> Option<String> {
    path.ancestors()
        .filter_map(|p| p.file_name()?.to_str())
        .map(|n| n.to_ascii_lowercase())
        .find(|n| is_bdf(n))
}

async fn read_pci_id_field(dev_dir: &std::path::Path, field: &str) -> Option<String> {
    let raw = tokio::fs::read_to_string(dev_dir.join(field)).await.ok()?;
    let trimmed = raw.trim();
    Some(
        trimmed
            .strip_prefix("0x")
            .unwrap_or(trimmed)
            .to_ascii_lowercase(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(v: &str, d: &str) -> DeviceId {
        DeviceId {
            vendor: v.into(),
            device: d.into(),
        }
    }

    fn entry(addr: &str, v: &str, d: &str) -> PassthroughEntry {
        PassthroughEntry {
            address: addr.into(),
            vendor: v.into(),
            device: d.into(),
        }
    }

    fn inv(addr: &str, v: &str, d: &str) -> PciInventoryEntry {
        PciInventoryEntry {
            address: addr.into(),
            vendor: v.into(),
            device: d.into(),
        }
    }

    #[test]
    fn vfio_id_renders_as_vendor_colon_device() {
        assert_eq!(id("10de", "2204").to_vfio_id(), "10de:2204");
    }

    #[test]
    fn bdf_format_validation() {
        assert!(is_bdf("0000:06:00.1"));
        assert!(is_bdf("0000:ff:1f.7"));
        assert!(!is_bdf("06:00.1")); // missing domain
        assert!(!is_bdf("0000:06:00")); // missing function
        assert!(!is_bdf("0000-06-00.1"));
        assert!(!is_bdf("zzzz:06:00.1"));
    }

    #[test]
    fn normalize_lowercases_and_sorts_dedupes() {
        let raw = vec![
            id("8086", "1539"),
            id("10DE", "2204"),
            id("10de", "2204"),
            id("8086", "1539"),
        ];
        let out = normalize_ids(raw);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0], id("10de", "2204"));
        assert_eq!(out[1], id("8086", "1539"));
    }

    #[test]
    fn normalize_drops_malformed_ids() {
        let raw = vec![
            id("10de", "2204"),
            id("zzzz", "0000"),
            id("10de", "abc"),
            id("10de", "12345"),
            id("8086", "1539"),
        ];
        let out = normalize_ids(raw);
        assert_eq!(out, vec![id("10de", "2204"), id("8086", "1539")]);
    }

    #[test]
    fn resolve_pairs_claims_every_matching_device() {
        // The legacy semantics being preserved: one pair, two identical
        // NICs → both claimed. This is exactly why pairs can't express
        // "this VF but not its siblings".
        let inventory = vec![
            inv("0000:06:00.0", "15b3", "1018"),
            inv("0000:06:00.1", "15b3", "1018"),
            inv("0000:07:00.0", "8086", "1539"),
        ];
        let out = resolve_pairs(&[id("15b3", "1018")], &inventory);
        assert_eq!(
            out.iter().map(|e| e.address.as_str()).collect::<Vec<_>>(),
            vec!["0000:06:00.0", "0000:06:00.1"]
        );
    }

    #[test]
    fn reconcile_migrates_legacy_ids_when_devices_empty() {
        let cfg = PassthroughConfig {
            devices: vec![],
            ids: vec![id("15b3", "1018")],
        };
        let inventory = vec![
            inv("0000:06:00.1", "15b3", "1018"),
            inv("0000:07:00.0", "8086", "1539"),
        ];
        let out = reconcile_legacy(cfg, &inventory);
        assert_eq!(out.devices, vec![entry("0000:06:00.1", "15b3", "1018")]);
    }

    #[test]
    fn reconcile_prefers_ids_when_rollback_edited_them() {
        // devices says the mlx VF; ids says an intel NIC — an old
        // engine changed the selection after a rollback. The pair set
        // is the fresher intent.
        let cfg = PassthroughConfig {
            devices: vec![entry("0000:06:00.1", "15b3", "1018")],
            ids: vec![id("8086", "1539")],
        };
        let inventory = vec![
            inv("0000:06:00.1", "15b3", "1018"),
            inv("0000:07:00.0", "8086", "1539"),
        ];
        let out = reconcile_legacy(cfg, &inventory);
        assert_eq!(out.devices, vec![entry("0000:07:00.0", "8086", "1539")]);
    }

    #[test]
    fn reconcile_keeps_consistent_state_untouched() {
        let cfg = PassthroughConfig {
            devices: vec![entry("0000:06:00.1", "15b3", "1018")],
            ids: vec![id("15b3", "1018")],
        };
        let out = reconcile_legacy(cfg, &[]);
        assert_eq!(out.devices, vec![entry("0000:06:00.1", "15b3", "1018")]);
    }

    #[test]
    fn derive_ids_dedupes_sibling_devices() {
        // Two VFs of the same card mirror to ONE legacy pair — a
        // rolled-back engine then claims both, which is the closest
        // the coarse mechanism can express.
        let ids = derive_ids(&[
            entry("0000:06:00.1", "15b3", "1018"),
            entry("0000:06:00.2", "15b3", "1018"),
        ]);
        assert_eq!(ids, vec![id("15b3", "1018")]);
    }

    #[test]
    fn render_udev_rules_emits_one_rule_per_device() {
        let cfg = PassthroughConfig {
            devices: vec![
                entry("0000:01:00.0", "1af4", "1041"),
                entry("0000:06:00.1", "15b3", "1018"),
            ],
            ids: vec![],
        };
        let rules = render_udev_rules(&cfg);
        assert!(
            rules.contains(r#"KERNEL=="0000:01:00.0", ATTR{driver_override}="vfio-pci""#),
            "{rules}"
        );
        assert!(rules.contains(r#"KERNEL=="0000:06:00.1""#), "{rules}");
        assert!(!rules.contains("vfio-pci.ids"), "{rules}");
    }

    #[test]
    fn render_udev_rules_empty_when_no_devices() {
        let rules = render_udev_rules(&PassthroughConfig::default());
        assert!(rules.is_empty());
    }

    #[test]
    fn bdf_resolution_walks_up_nested_buses() {
        use std::path::Path;
        // virtio NIC: device symlink lands on the virtio-bus child.
        assert_eq!(
            bdf_from_device_path(Path::new(
                "/sys/devices/pci0000:00/0000:00:05.0/0000:07:12.0/virtio3"
            )),
            Some("0000:07:12.0".to_string())
        );
        // Plain PCI NIC: symlink lands on the function itself.
        assert_eq!(
            bdf_from_device_path(Path::new("/sys/devices/pci0000:00/0000:06:00.1")),
            Some("0000:06:00.1".to_string())
        );
        // Non-PCI (USB NIC deep path without BDF): none.
        assert_eq!(
            bdf_from_device_path(Path::new("/sys/devices/virtual/net/x")),
            None
        );
    }

    #[tokio::test]
    async fn validate_passes_when_mgmt_iface_unknown() {
        let res = validate_request(&[entry("0000:06:00.1", "15b3", "1018")], None).await;
        assert!(res.is_ok());
    }

    #[tokio::test]
    async fn validate_passes_when_mgmt_iface_has_no_pci_device() {
        let res = validate_request(
            &[entry("0000:06:00.1", "15b3", "1018")],
            Some("nonexistent-iface-zzz"),
        )
        .await;
        assert!(res.is_ok());
    }
}
