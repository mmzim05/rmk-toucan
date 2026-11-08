//! Keymap and key behaviors, converted from zmk-config-toucan's
//! `config/toucan.keymap`. Vial edits are stored on the keyboard and take
//! precedence over this default.

use rmk::config::BehaviorConfig;
use rmk::heapless::Vec;
use rmk::keyboard::combo::{Combo, ComboConfig};
use rmk::types::action::KeyAction;
use rmk::types::modifier::ModifierCombination as Mods;
use rmk::types::morse::{MorseMode, MorseProfile};
use rmk::{a, k, lt, ltp, mo, shifted, thp, to, user, wm};

pub const ROW: usize = 4;
pub const COL: usize = 12;
pub const NUM_LAYER: usize = 5;

// Tap-hold profiles, by index into `MorsesConfig::profiles`
/// ZMK "balanced" flavor, 200ms
const BAL: u8 = 0;
/// ZMK "hold-preferred", 300ms, quick-tap 150ms
const HP: u8 = 1;
/// Touch key: held past 150ms = PAD layer, shorter touch = left click
const TP: u8 = 2;

pub fn behavior_config() -> BehaviorConfig {
    let mut config = BehaviorConfig::default();
    // Plain `lt!` keys behave like ZMK's default &lt: tap-preferred, 200ms
    config.morse.default_profile = MorseProfile::new(None, Some(MorseMode::Normal), Some(200), Some(200));
    let profiles = [
        MorseProfile::new(None, Some(MorseMode::PermissiveHold), Some(200), None),
        MorseProfile::new(None, Some(MorseMode::HoldOnOtherPress), Some(300), None)
            .with_quick_tap_timeout_ms(Some(150)),
        MorseProfile::new(None, Some(MorseMode::HoldOnOtherPress), Some(150), None),
    ];
    config.morse.profiles = Vec::from_slice(&profiles).unwrap();

    // ZMK combos GAME (positions 3 13 14 16) and TYPE (positions 19-22)
    config.combo.combos[0] = Some(Combo::new(ComboConfig::new([k!(D), k!(N), k!(R), k!(S)], to!(4), None)));
    config.combo.combos[1] = Some(Combo::new(ComboConfig::new([k!(A), k!(H), k!(E), k!(I)], to!(0), None)));
    config
}

const NO: KeyAction = a!(No);
const __: KeyAction = a!(Transparent);

#[rustfmt::skip]
pub const fn default_keymap() -> [[[KeyAction; COL]; ROW]; NUM_LAYER] {
    [
        // BASE
        [
            [ltp!(1, Escape, BAL), k!(B), k!(L), k!(D), k!(C), k!(V), k!(J), k!(Y), k!(O), k!(U), k!(Comma), k!(Backslash)],
            [k!(LCtrl), k!(N), k!(R), k!(T), k!(S), k!(G), k!(P), k!(H), k!(A), k!(E), k!(I), k!(Minus)],
            [k!(LAlt), k!(X), k!(Q), k!(M), k!(W), k!(Z), k!(K), k!(F), k!(Quote), k!(Semicolon), k!(Dot), k!(Slash)],
            [NO, NO, NO, k!(Backspace), k!(LShift), thp!(Delete, LGui, HP), lt!(3, Enter), lt!(2, Space), k!(Tab), ltp!(1, MouseBtn1, TP), NO, NO],
        ],
        // PAD
        [
            [__, wm!(Kc1, Mods::LGUI), wm!(Kc2, Mods::LGUI), wm!(Kc3, Mods::LGUI), wm!(Kc4, Mods::LGUI), wm!(Kc5, Mods::LGUI), wm!(Kc6, Mods::LGUI), wm!(Kc7, Mods::LGUI), wm!(Kc8, Mods::LGUI), wm!(Kc9, Mods::LGUI), wm!(Kc0, Mods::LGUI), __],
            [__, mo!(3), k!(MouseBtn3), k!(MouseBtn2), k!(MouseBtn1), __, k!(MouseBtn1), k!(MouseBtn2), k!(MouseBtn3), __, __, __],
            [__, __, __, __, __, __, k!(MouseBtn1), k!(MouseBtn2), k!(MouseBtn3), __, __, __],
            [NO, NO, NO, __, __, __, __, __, __, __, NO, NO],
        ],
        // NUM
        [
            [__, k!(Grave), shifted!(Grave), shifted!(Equal), k!(Equal), __, __, shifted!(LeftBracket), shifted!(RightBracket), k!(LeftBracket), k!(RightBracket), __],
            [__, k!(Kc1), k!(Kc2), k!(Kc3), k!(Kc4), k!(Kc5), k!(Kc6), k!(Kc7), k!(Kc8), k!(Kc9), k!(Kc0), __],
            [__, shifted!(Kc1), shifted!(Kc2), shifted!(Kc3), shifted!(Kc4), shifted!(Kc5), shifted!(Kc6), shifted!(Kc7), shifted!(Kc8), shifted!(Kc9), shifted!(Kc0), __],
            [NO, NO, NO, __, k!(Comma), k!(Dot), __, __, __, __, NO, NO],
        ],
        // NAV. Wireless keys (5 BLE profiles): User0-4 select a profile (hold
        // 5s to clear it), User7 clears the current profile, User10 switches
        // to the dongle (hold 5s to pair a different dongle).
        [
            [user!(7), user!(0), user!(1), user!(2), user!(3), user!(4), k!(Insert), k!(PageDown), k!(PageUp), k!(AudioMute), k!(AudioVolUp), k!(AudioVolDown)],
            [k!(LCtrl), user!(10), k!(MediaPrevTrack), k!(MediaPlayPause), k!(MediaNextTrack), NO, k!(Left), k!(Down), k!(Up), k!(Right), k!(Home), k!(End)],
            [k!(F1), k!(F2), k!(F3), k!(F4), k!(F5), k!(F6), k!(F7), k!(F8), k!(F9), k!(F10), k!(F11), k!(F12)],
            [NO, NO, NO, k!(LAlt), k!(LShift), k!(LGui), __, __, __, __, NO, NO],
        ],
        // GAME: trackpad off (see pointing_mode.rs), touch key does nothing
        [
            [k!(Escape), k!(B), k!(L), k!(D), k!(C), k!(V), k!(J), k!(Y), k!(O), k!(U), k!(Comma), k!(Backslash)],
            [k!(LCtrl), k!(N), k!(R), k!(T), k!(S), k!(G), k!(P), k!(H), k!(A), k!(E), k!(I), k!(Minus)],
            [k!(LAlt), k!(X), k!(Q), k!(M), k!(W), k!(Z), k!(K), k!(F), k!(Quote), k!(Semicolon), k!(Dot), k!(Slash)],
            [NO, NO, NO, k!(Left), k!(Space), k!(Right), lt!(2, Enter), lt!(2, Space), k!(Tab), NO, NO, NO],
        ],
    ]
}
