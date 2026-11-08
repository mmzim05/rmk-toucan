#!/usr/bin/env bash
# Flash the left half, the right half, then the dongle, see flash.sh
set -euo pipefail
cd "$(dirname "$0")"
./flash.sh left
./flash.sh right
./flash.sh dongle
