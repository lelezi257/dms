#!/usr/bin/env bash
set -euo pipefail
kind=$1
case "$kind" in
 a) run=/mnt/lima-afsadata/afs-delivery/round2-eio-v80-r2 ;;
 ctl) run=/mnt/lima-afsctlstate/afs-delivery/round2-eio-v80-r2 ;;
 b) run=/mnt/lima-afsbdata/afs-delivery/owner-eio-v80-r2-b ;;
 *) exit 2;;
esac
out=/home/lzc.guest/eio-v80-r2-$kind.tar.gz
test ! -e "$out"
sudo tar -czf "$out" -C "$run" evidence etc log
sudo chown 501:501 "$out"
