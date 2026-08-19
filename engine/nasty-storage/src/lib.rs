//! btrfs filesystem and subvolume management
//!
//! This crate wraps btrfs-progs CLI (`mkfs.btrfs`, `mount -t btrfs`,
//! `btrfs scrub` / `subvolume`) for filesystem lifecycle operations.

pub mod cmd;
pub mod disk_type;
pub mod filesystem;
pub mod io_scheduler;
pub mod subvolume;

pub use filesystem::{FilesystemError, FilesystemService};
pub use subvolume::SubvolumeService;
