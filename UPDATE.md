# Updating NASty (Debian)

Updates are applied from the WebUI (**Update** tab) or via apt. Both paths
use **snapper** on the btrfs root filesystem so you can roll back.

## WebUI

1. Open **Update** → check for updates → **Upgrade**
2. The engine runs `apt-get update` / `apt-get upgrade` in a transient
   systemd unit (`nasty-update.service`)
3. apt hooks create snapper **pre** / **post** snapshots automatically
4. Reboot if the UI reports reboot required (new kernel, snapper rollback)

## Generations (snapper)

The **Generations** tab lists snapper snapshots for config `root`.

- **Switch** → `snapper -c root rollback <n>` then reboot
- **Delete** → `snapper -c root delete <n>`
- **Label** → stored in `/var/lib/nasty/generation-labels.json` and
  snapper description when possible

## Manual apt upgrade

```bash
sudo apt-get update
sudo apt-get upgrade
snapper -c root list
```

## Manual rollback

```bash
snapper -c root list
sudo snapper -c root --ambit classic rollback <snapshot-number>
sudo reboot
```

## Channels

Release channel (mild / spicy / nasty) still selects which GitHub release
tags the Version page compares against. Package installation itself always
goes through apt candidates for `nasty`, `nasty-engine`, and `nasty-webui`.

## Recovery without WebUI

If the engine will not start after an upgrade:

```bash
snapper -c root list
sudo snapper -c root --ambit classic rollback <good-snapshot>
sudo reboot
```

Or boot a recovery environment, mount the btrfs root, and set the default
subvolume to a known-good snapshot with `btrfs subvolume set-default`.
