#!/bin/bash
# Helper invoked from /etc/apt/apt.conf.d/80nasty-snapper
set -euo pipefail
MODE="${1:-}"
command -v snapper >/dev/null 2>&1 || exit 0
snapper -c root get-config >/dev/null 2>&1 || exit 0

case "$MODE" in
  pre)
    snapper -c root create --type pre --cleanup-algorithm number \
      --description "apt $(date -Is)" --userdata "nasty=apt-pre"
    # Remember pre number for pairing (best-effort).
    snapper -c root list --columns number,type 2>/dev/null \
      | awk '/pre/ {n=$1} END {print n}' > /run/nasty-apt-snapper-pre 2>/dev/null || true
    ;;
  post)
    PRE=""
    if [ -f /run/nasty-apt-snapper-pre ]; then
      PRE=$(cat /run/nasty-apt-snapper-pre || true)
      rm -f /run/nasty-apt-snapper-pre
    fi
    if [ -n "${PRE:-}" ]; then
      snapper -c root create --type post --pre-number "$PRE" --cleanup-algorithm number \
        --description "apt $(date -Is)" --userdata "nasty=apt-post" || true
    else
      snapper -c root create --cleanup-algorithm number \
        --description "apt post $(date -Is)" --userdata "nasty=apt-post" || true
    fi
    ;;
  *)
    exit 0
    ;;
esac
