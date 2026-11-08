#!/usr/bin/env bash
# Build all three Toucan firmwares and package them as UF2 files in ./firmware.
#
# Needs: the thumbv7em-none-eabihf target and llvm-tools rustup component,
# cargo-hex-to-uf2 (`cargo install cargo-hex-to-uf2`), and arm-none-eabi-gcc
# with newlib for the BLE crypto C code (Arch: arm-none-eabi-gcc
# arm-none-eabi-newlib; Debian/Ubuntu: gcc-arm-none-eabi libnewlib-dev).
set -euo pipefail
cd "$(dirname "$0")"

objcopy="$(rustc --print sysroot)/lib/rustlib/$(rustc -vV | sed -n 's/^host: //p')/bin/llvm-objcopy"
mkdir -p firmware

package() { # <crate dir> <bin> <output name>
    local elf="$1/target/thumbv7em-none-eabihf/release/$2"
    "$objcopy" -O ihex "$elf" "firmware/$3.hex"
    cargo hex-to-uf2 --input-path "firmware/$3.hex" --output-path "firmware/$3.uf2" --family nrf52840
    rm "firmware/$3.hex"
}

(cd keyboard && cargo build --release)
(cd dongle && cargo build --release)
package keyboard central toucan_left
package keyboard peripheral toucan_right
package dongle dongle toucan_dongle
ls -l firmware
