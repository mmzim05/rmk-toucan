# rmk-toucan

[RMK](https://github.com/rmk-rs/rmk) firmware for the [beekeeb Toucan](https://beekeeb.com/toucan-keyboard/),
converted from [zmk-config-toucan](https://github.com/mmzim05/zmk-config-toucan). All three boards are
Seeed XIAO nRF52840.

| Board | Firmware | Role |
| --- | --- | --- |
| Left half | `toucan_left.uf2` | Split central: keymap, status display, talks to the host |
| Right half | `toucan_right.uf2` | Split peripheral: Cirque trackpad, touch key, inertial cursor |
| Dongle | `toucan_dongle.uf2` | USB dongle: relays the left half's reports and Vial to the host |

The left half can also connect to a host directly over BLE (5 profiles) or USB.

## What's where

- `keyboard/src/keymap.rs`: keymap, tap-hold profiles, combos (same layers as the ZMK keymap: BASE, PAD,
  NUM, NAV, GAME).
- `keyboard/src/central.rs`: left half, written with RMK's Rust API.
- `keyboard/src/peripheral.rs` + `keyboard/keyboard.toml`: right half, generated from the TOML.
- `keyboard/src/trackpad.rs`: Cirque Pinnacle driver and the gesture logic from `zmk-input-gestures`
  (touch key, 30° rotation, inertial glide). Tuning constants are at the top of the file.
- `keyboard/src/pointing_mode.rs`: trackpad per layer: cursor at 75% speed, scroll on NAV, off on GAME.
- `keyboard/src/display.rs`: Sharp memory-LCD driver and the status screen (left battery, right battery,
  active layer).
- `dongle/`: the dongle.

## Build

CI builds on every push and uploads the three `.uf2` files as the `firmware` artifact.

Locally:

```sh
sudo pacman -S arm-none-eabi-gcc arm-none-eabi-newlib   # C code in RMK's BLE crypto
rustup target add thumbv7em-none-eabihf
rustup component add llvm-tools
cargo install cargo-hex-to-uf2
./build.sh                                              # writes firmware/*.uf2
```

## Flash and pair

1. Run `./flash-all.sh` (left, right, dongle in that order) or `./flash-left.sh`, `./flash-right.sh`,
   `./flash-dongle.sh`. Each asks you to double-tap reset on the board, waits for its `XIAO-BOOT` drive,
   copies the `.uf2` from `firmware/` and checks that the board rebooted. By hand: double-tap reset and
   copy the `.uf2` to the drive that appears.
2. The halves find each other on their own.
3. Plug in the dongle, hold the NAV layer key (right thumb, Enter on tap) and press the left home-row
   pinky key (N on BASE; `User10`, "switch to dongle"). The dongle pairs and both reconnect on
   their own from then on. Hold that key for 5 seconds to pair a different dongle.

RMK uses the flash region where the XIAO's bootloader keeps its SoftDevice. To go back to ZMK, build ZMK
without the SoftDevice reservation (`board: xiao_ble//zmk`), see
[RMK's FAQ](https://github.com/rmk-rs/rmk/blob/main/docs/docs/main/docs/getting_started/faq.md).

## Live keymap edits

Vial works over USB, BLE, or through the dongle. To unlock it, hold the left G key (row 1, col 5) when Vial asks.
