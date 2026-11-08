//! Trackpad behavior per layer, on the central. Replaces the ZMK
//! `glidepoint_listener` input-processor chain:
//! - everywhere: cursor at 75% speed (`zip_xy_scaler 75 100`)
//! - NAV (layer 3): scroll, horizontal reversed
//! - GAME (layer 4): trackpad off

use rmk::event::{LayerChangeEvent, PointingProcessorEvent, publish_event};
use rmk::input_device::pointing::{ALL_POINTING_DEVICES, CursorConfig, PointingMode, ScrollAxis, ScrollConfig};
use rmk::macros::processor;

const NAV: u8 = 3;
const GAME: u8 = 4;

/// Trackpad motion is in 1/1024ths of the pad per axis.
pub const CURSOR: PointingMode = PointingMode::Cursor(CursorConfig {
    multiplier_x: 3,
    multiplier_y: 3,
    divisor_x: 4,
    divisor_y: 4,
    invert_x: false,
    invert_y: false,
});

/// Pad motion per wheel step. Raise `divisor` to scroll slower.
const SCROLL: PointingMode = PointingMode::Scroll(ScrollConfig {
    // Finger down scrolls down, as with ZMK's REL_Y -> WHEEL mapping
    wheel: ScrollAxis {
        multiplier: 1,
        divisor: 16,
        invert: true,
    },
    // ZMK applied INPUT_TRANSFORM_X_INVERT to horizontal scroll
    pan: ScrollAxis {
        multiplier: 1,
        divisor: 16,
        invert: true,
    },
});

const OFF: PointingMode = PointingMode::Cursor(CursorConfig {
    multiplier_x: 0,
    multiplier_y: 0,
    divisor_x: 1,
    divisor_y: 1,
    invert_x: false,
    invert_y: false,
});

#[processor(subscribe = [LayerChangeEvent])]
pub struct PointingModeByLayer;

impl PointingModeByLayer {
    async fn on_layer_change_event(&mut self, event: LayerChangeEvent) {
        let mode = match event.0 {
            NAV => SCROLL,
            GAME => OFF,
            _ => CURSOR,
        };
        publish_event(PointingProcessorEvent {
            device_id: ALL_POINTING_DEVICES,
            mode,
        });
    }
}
