#!/usr/bin/env bash
# Flash one Toucan board over the XIAO's UF2 bootloader.
#
#   ./flash.sh left|right|dongle
#
# Uses the files ./build.sh writes to firmware/. Prompts you to put the board
# into the bootloader (double-tap reset), waits for its XIAO-BOOT drive, copies
# the .uf2 and waits for the board to reboot into the new firmware.
set -euo pipefail
cd "$(dirname "$0")"

LABEL=XIAO-BOOT
DEV=/dev/disk/by-label/$LABEL
TIMEOUT=30

case "${1:-}" in
left) name="left half" ;;
right) name="right half" ;;
dongle) name="dongle" ;;
*)
    echo "usage: $0 left|right|dongle" >&2
    exit 2
    ;;
esac
uf2="firmware/toucan_$1.uf2"
[[ -f $uf2 ]] || {
    echo "$uf2 not found, run ./build.sh first" >&2
    exit 1
}

# Wait up to $TIMEOUT seconds for a command to succeed
wait_for() {
    local i
    for ((i = 0; i < TIMEOUT * 2; i++)); do
        "$@" && return 0
        sleep 0.5
    done
    return 1
}

mountpoint_of() { findmnt -n -o TARGET --source "$(readlink -f "$DEV")" 2>/dev/null | head -n1; }
is_mounted() { [[ -e $DEV && -n $(mountpoint_of) ]]; }
is_gone() { [[ ! -e $DEV ]]; }

echo
echo "== Flashing the $name ($uf2)"
read -rp "Connect the $name over USB, double-tap its reset button, then press Enter... "

echo "Waiting for the $LABEL drive..."
wait_for test -e "$DEV" || {
    echo "No $LABEL drive after ${TIMEOUT}s. Is the board in the bootloader?" >&2
    exit 1
}

# Mount it ourselves if no automounter did
if ! wait_for is_mounted; then
    udisksctl mount --no-user-interaction -b "$(readlink -f "$DEV")" >/dev/null
fi
mnt=$(mountpoint_of)
[[ -n $mnt ]] || {
    echo "Could not mount $DEV" >&2
    exit 1
}

grep -q "nRF52840" "$mnt/INFO_UF2.TXT" 2>/dev/null || {
    echo "$mnt does not look like a XIAO nRF52840 bootloader, not flashing" >&2
    exit 1
}

echo "Copying to $mnt..."
# The bootloader reboots as soon as the last block lands, so the tail of the
# write and the sync can fail with I/O errors. That is expected.
cp "$uf2" "$mnt/" 2>/dev/null || true
sync 2>/dev/null || true

if wait_for is_gone; then
    echo "Done: the $name rebooted into the new firmware."
else
    echo "The $LABEL drive is still there after ${TIMEOUT}s; the flash probably failed." >&2
    exit 1
fi
