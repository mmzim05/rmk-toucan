# rmk-toucan

RMK port of `~/Development/github/toucan/zmk-config-toucan` (see its `CLAUDE.md` for the hardware and the
ZMK-era tuning history). README covers layout of the repo, building and pairing.

## Architecture

- Left half = split central (Rust API, `keyboard/src/central.rs`), right half = peripheral (TOML macro,
  `keyboard/src/peripheral.rs`), dongle = RMK `dongle` relay (`dongle/`). Unlike ZMK, the dongle is not
  the split central, so the left half knows the layer and both batteries natively.
- RMK is pinned to one git rev in both `Cargo.toml`s; keep them equal.

## Gotchas

- Why the central is Rust API: `#[register_processor]` bodies in the TOML macro run before the keymap is
  built, and `PointingProcessor` needs `&keymap` (for held mouse buttons during drags).
- The peripheral macro ignores `add_interrupt!`, so `SPI2` is bound in a separate `bind_interrupts!`
  (`SpiIrqs`).
- `keyboard.toml` still feeds RMK's build constants for the central (BLE profiles, event channel sizes,
  debounce, sleep timeout). Pins/offsets in `[split.central]` are informational and must match
  `central.rs`.
- Extra event subscribers must be declared: the pointing-mode controller is the `+1` in
  `[event.layer_change]`.
- Building needs `arm-none-eabi-gcc` + newlib (`p256-cortex-m4-sys`).
- The SDC defaults to one peripheral link: every `build_sdc` must set `central_count` and
  `peripheral_count` to match the counts passed to `calculate_sdc_memory_size`, or `build` fails and the
  board dies at boot (the dongle did, silently). The dongle's panic handler lights the red LED.

## Hardware status

Working on the keyboard: Sharp LCD, Pinnacle trackpad, split link, USB, dongle.

- Battery: the SAADC needs 40us acquisition for the XIAO's 1M/510k divider (10us default reads low).
  The right half's generated ADC gets it from `BatteryAdcAcqTime` in `peripheral.rs`.
- Trackpad tuning (`trackpad.rs` constants) was done by feel with the user: expo, decay, Z hysteresis
  (`Z_TOUCH_START`/`Z_TOUCH_KEEP` are still guesses, no Z readings measured), swipe-vs-tap
  (`TAP_MAX_STEP`). The pad stays at 100 Hz; it beats against the 7.5ms split link interval, the user
  preferred that over 200 Hz.
- Not verified: scroll divisor (16).
