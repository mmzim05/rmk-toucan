//! Cirque Pinnacle trackpad on the right half, with the touch key and the
//! inertial cursor.
//!
//! Ported from the ZMK setup: the chip init follows `input_pinnacle.c`
//! (geeksville/cirque-input-module, `toucan` branch), and the gesture logic
//! follows `peripheral_gesture.c` (zmk-input-gestures). The pad runs in
//! absolute mode; this side turns positions into relative motion, so only
//! small REL deltas cross the split link.
//!
//! Outputs, all forwarded to the central by RMK's split peripheral:
//! - `PointingEvent` with REL X/Y, while the finger moves and while the
//!   cursor glides after a fast lift.
//! - `KeyboardEvent` for the touch key at local (3, 3) = keymap (3, 9),
//!   pressed while a finger rests on the pad.

use defmt::{error, info, warn};
use embassy_time::{Duration, Instant, Timer};
use embedded_hal::digital::OutputPin;
use embedded_hal_async::digital::Wait;
use embedded_hal_async::spi::SpiBus;
use rmk::core_traits::Runnable;
use rmk::embassy_futures::select::{Either, select};
use rmk::event::{Axis, AxisEvent, AxisValType, KeyboardEvent, PointingEvent, publish_event_async};

// --- Gesture tuning, from `periph_gesture` in toucan_right.overlay. The pad
// samples at its 100 Hz default.

/// Clamp per-sample jumps (scaled pad units). Scales the whole step, so a
/// clamped step keeps its direction. ZMK used 60; raised so fast flicks
/// aren't capped.
const MAX_DELTA: i32 = 90;
/// Fallback lift detection if the pad's lift (Z = 0) packet is missed: this
/// much silence counts as lifted. At least 2x the ~10ms sample interval, so
/// one or two missed samples don't split a drag.
const TOUCH_TIMEOUT: Duration = Duration::from_millis(30);
/// Minimum lift speed for a glide, in tenths of a unit per ms.
const VELOCITY_THRESHOLD: u32 = 3;
/// Per mille of glide speed lost per inertial frame (ZMK used 60). Lower
/// glides longer.
const DECAY_PER_MILLE: i32 = 100;
/// Glide start speed, in percent of the lift speed.
const SPEED_SCALE: i64 = 100;
/// Samples in a row needed before a touch counts, to drop single-sample
/// phantom touches from electrical noise.
const TOUCH_CONFIRM_SAMPLES: u8 = 2;
/// Virtual rotation for the pad's physical tilt: 30 degrees CCW, as Q8
/// (value * 256) sine and cosine.
const ROTATE_SIN_Q8: i32 = 128;
const ROTATE_COS_Q8: i32 = 221;
/// Inertial frame interval, matching the pad's ~100 Hz sample rate.
const ANIMATE_MS: u64 = 10;
/// Cursor expo: each step is scaled by a gain that grows with the step's
/// length (its longer axis, pad units per sample): `EXPO_MIN` for the
/// slowest moves, 1x at `EXPO_UNITY` units, rising at the same rate up to
/// `EXPO_MAX` for fast swipes. Gains are in 1/256ths. Lower `EXPO_MIN` for
/// more precise slow moves, raise `EXPO_MAX` for a higher top speed.
const EXPO_MIN_Q8: i32 = 141; // 0.55x
const EXPO_UNITY: i32 = 12;
const EXPO_MAX_Q8: i32 = 410; // 1.6x
/// Number of recent samples the lift speed is averaged over. The newest one
/// is left out when there are enough (see `on_touch_end`).
const VEL_WINDOW: usize = 5;

/// Taps on the touch key click (keymap: `ltp!(1, MouseBtn1, TP)`, tapped
/// under 150ms). A touch whose fastest step (longer axis, pad units per
/// sample) went over this is a swipe, not a tap: its release is held back
/// to `SWIPE_RELEASE_AFTER` past the press, so the keymap sees a hold of
/// the PAD layer with no other key, which sends nothing.
const TAP_MAX_STEP: i32 = 8;
/// Past the touch key's 150ms tap timeout (profile `TP` in keymap.rs).
const SWIPE_RELEASE_AFTER: Duration = Duration::from_millis(180);

/// Touch strength (Z) hysteresis, against a finger hovering just above the
/// pad. A touch starts once Z reaches `Z_TOUCH_START` and ends (like a lift)
/// when Z drops below `Z_TOUCH_KEEP`. The gap keeps light contact mid-drag
/// from dropping out (why the ADC stays at its most sensitive, 1x).
/// Hovering still moves the cursor: raise `Z_TOUCH_START`, or `Z_TOUCH_KEEP`
/// if it happens after lifting. Light touches missed or drags breaking up:
/// lower them.
const Z_TOUCH_START: u8 = 16;
const Z_TOUCH_KEEP: u8 = 4;

/// Touch key position in the right half's own matrix.
const TOUCH_KEY_ROW: u8 = 3;
const TOUCH_KEY_COL: u8 = 3;

const POINTING_DEVICE_ID: u8 = 0;

// --- Pinnacle registers ---

const READ: u8 = 0xA0;
const WRITE: u8 = 0x80;
const AUTOINC: u8 = 0xFC;
const FILLER: u8 = 0xFB;

const FW_ID: u8 = 0x00;
const STATUS1: u8 = 0x02;
const STATUS1_SW_DR: u8 = 1 << 2;
const SYS_CFG: u8 = 0x03;
const SYS_CFG_RESET: u8 = 1 << 0;
const FEED_CFG1: u8 = 0x04;
const FEED_CFG1_EN_FEED: u8 = 1 << 0;
const FEED_CFG1_ABS_MODE: u8 = 1 << 1;
const FEED_CFG1_INV_X: u8 = 1 << 6;
const FEED_CFG1_INV_Y: u8 = 1 << 7;
const FEED_CFG2: u8 = 0x05;
const FEED_CFG2_EN_IM: u8 = 1 << 0;
const FEED_CFG2_EN_BTN_SCRL: u8 = 1 << 6;
const CAL_CFG: u8 = 0x07;
const Z_IDLE: u8 = 0x0A;
const PACKET0: u8 = 0x12;
const ERA_VALUE: u8 = 0x1B;
const ERA_HIGH_BYTE: u8 = 0x1C;
const ERA_LOW_BYTE: u8 = 0x1D;
const ERA_CONTROL: u8 = 0x1E;
const ERA_CONTROL_READ: u8 = 0x01;
const ERA_CONTROL_WRITE: u8 = 0x02;

const ERA_X_AXIS_WIDE_Z_MIN: u16 = 0x0149;
const ERA_Y_AXIS_WIDE_Z_MIN: u16 = 0x0168;
const ERA_TRACKING_ADC_CONFIG: u16 = 0x0187;

/// ADC attenuation "1x", the most sensitive setting. Fewer Z dropouts under
/// light contact, so a drag doesn't break into a click.
const ADC_SENSITIVITY_1X: u8 = 0x00;
const X_AXIS_Z_MIN: u8 = 5;
const Y_AXIS_Z_MIN: u8 = 4;

/// Active area per the datasheet, scaled to 0..1024 on both axes.
const CLAMP_X: (i32, i32) = (128, 1920);
const CLAMP_Y: (i32, i32) = (64, 1472);
const SCALE_TO: i32 = 1024;

pub struct Trackpad<S, CS, DR> {
    spi: S,
    cs: CS,
    dr: DR,
    gesture: Gesture,
}

impl<S: SpiBus, CS: OutputPin, DR: Wait> Trackpad<S, CS, DR> {
    /// `spi` must be in SPI mode 1, at most 13 MHz. `cs` is active low.
    /// `dr` is the data-ready pin, high while a sample is waiting.
    pub fn new(spi: S, cs: CS, dr: DR) -> Self {
        Self {
            spi,
            cs,
            dr,
            gesture: Gesture::new(),
        }
    }

    async fn read_regs(&mut self, addr: u8, buf: &mut [u8]) -> Result<(), S::Error> {
        let n = buf.len() + 3;
        let mut tx = [AUTOINC; 9];
        let mut rx = [0u8; 9];
        tx[0] = READ | addr;
        self.cs.set_low().ok();
        let res = self.spi.transfer(&mut rx[..n], &tx[..n]).await;
        self.cs.set_high().ok();
        res?;
        buf.copy_from_slice(&rx[3..n]);
        Ok(())
    }

    async fn read_reg(&mut self, addr: u8) -> Result<u8, S::Error> {
        let mut v = [0u8];
        self.read_regs(addr, &mut v).await?;
        Ok(v[0])
    }

    async fn write_reg(&mut self, addr: u8, val: u8) -> Result<(), S::Error> {
        let mut rx = [0u8; 2];
        self.cs.set_low().ok();
        let res = self.spi.transfer(&mut rx, &[WRITE | addr, val]).await;
        self.cs.set_high().ok();
        res?;
        if rx[1] != FILLER {
            warn!("pinnacle: unexpected write reply {:#x}", rx[1]);
        }
        Timer::after_micros(50).await;
        Ok(())
    }

    async fn era_wait(&mut self) -> Result<(), S::Error> {
        while self.read_reg(ERA_CONTROL).await? != 0 {}
        Ok(())
    }

    async fn era_read(&mut self, addr: u16) -> Result<u8, S::Error> {
        self.write_reg(ERA_HIGH_BYTE, (addr >> 8) as u8).await?;
        self.write_reg(ERA_LOW_BYTE, addr as u8).await?;
        self.write_reg(ERA_CONTROL, ERA_CONTROL_READ).await?;
        self.era_wait().await?;
        let v = self.read_reg(ERA_VALUE).await?;
        self.write_reg(STATUS1, 0).await?;
        Ok(v)
    }

    async fn era_write(&mut self, addr: u16, val: u8) -> Result<(), S::Error> {
        self.write_reg(ERA_VALUE, val).await?;
        self.write_reg(ERA_HIGH_BYTE, (addr >> 8) as u8).await?;
        self.write_reg(ERA_LOW_BYTE, addr as u8).await?;
        self.write_reg(ERA_CONTROL, ERA_CONTROL_WRITE).await?;
        self.era_wait().await?;
        self.write_reg(STATUS1, 0).await
    }

    async fn init(&mut self) -> Result<(), S::Error> {
        let mut id = [0u8; 2];
        self.read_regs(FW_ID, &mut id).await?;
        info!("pinnacle: fw id {:#x} version {:#x}", id[0], id[1]);
        Timer::after_millis(10).await;
        self.write_reg(STATUS1, 0).await?;
        self.write_reg(SYS_CFG, SYS_CFG_RESET).await?;
        Timer::after_millis(20).await;
        self.write_reg(Z_IDLE, 5).await?;

        let adc = self.era_read(ERA_TRACKING_ADC_CONFIG).await?;
        self.era_write(ERA_TRACKING_ADC_CONFIG, (adc & 0x3F) | ADC_SENSITIVITY_1X)
            .await?;
        self.era_write(ERA_X_AXIS_WIDE_Z_MIN, X_AXIS_Z_MIN).await?;
        self.era_write(ERA_Y_AXIS_WIDE_Z_MIN, Y_AXIS_Z_MIN).await?;

        // Force a recalibration and wait for it to finish
        let cal = self.read_reg(CAL_CFG).await?;
        self.write_reg(CAL_CFG, cal | 0x01).await?;
        while self.read_reg(CAL_CFG).await? & 0x01 != 0 {}

        self.write_reg(FEED_CFG2, FEED_CFG2_EN_IM | FEED_CFG2_EN_BTN_SCRL).await?;
        self.write_reg(
            FEED_CFG1,
            FEED_CFG1_EN_FEED | FEED_CFG1_ABS_MODE | FEED_CFG1_INV_X | FEED_CFG1_INV_Y,
        )
        .await?;
        self.write_reg(STATUS1, 0).await
    }

    /// Reads one absolute sample and clears data-ready.
    async fn read_sample(&mut self) -> Result<Sample, S::Error> {
        if self.read_reg(STATUS1).await? & STATUS1_SW_DR == 0 {
            return Ok(Sample::None);
        }
        let mut p = [0u8; 6];
        self.read_regs(PACKET0, &mut p).await?;
        self.write_reg(STATUS1, 0).await?;

        let x = (((p[4] & 0x0F) as i32) << 8) | p[2] as i32;
        let y = (((p[4] & 0xF0) as i32) << 4) | p[3] as i32;
        let z = p[5] & 0x3F;
        if z == 0 {
            return Ok(Sample::Lift);
        }
        let x = (x.clamp(CLAMP_X.0, CLAMP_X.1) - CLAMP_X.0) * SCALE_TO / (CLAMP_X.1 - CLAMP_X.0);
        let y = (y.clamp(CLAMP_Y.0, CLAMP_Y.1) - CLAMP_Y.0) * SCALE_TO / (CLAMP_Y.1 - CLAMP_Y.0);
        Ok(Sample::Touch(x, y, z))
    }
}

enum Sample {
    /// Finger down at this scaled position, with touch strength Z
    Touch(i32, i32, u8),
    /// The pad's lift packet (Z = 0)
    Lift,
    /// Data-ready was not set
    None,
}

impl<S: SpiBus, CS: OutputPin, DR: Wait> Runnable for Trackpad<S, CS, DR> {
    async fn run(&mut self) -> ! {
        self.cs.set_high().ok();
        while self.init().await.is_err() {
            error!("pinnacle: init failed, retrying");
            Timer::after_secs(1).await;
        }

        loop {
            let deadline = [self.gesture.touch_end_at, self.gesture.glide_at, self.gesture.release_at]
                .into_iter()
                .flatten()
                .min();
            let timer = async {
                match deadline {
                    Some(t) => Timer::at(t).await,
                    None => core::future::pending().await,
                }
            };
            match select(self.dr.wait_for_high(), timer).await {
                Either::First(_) => match self.read_sample().await {
                    Ok(Sample::Touch(x, y, z)) => {
                        let in_touch = self.gesture.touch_end_at.is_some();
                        if z >= Z_TOUCH_START || (in_touch && z >= Z_TOUCH_KEEP) {
                            self.gesture.on_sample(x, y).await
                        } else if in_touch {
                            // Finger lifted into a hover
                            self.gesture.on_touch_end().await;
                        }
                    }
                    // Start the glide right away instead of waiting for the
                    // touch timeout, so the cursor doesn't pause at lift
                    Ok(Sample::Lift) => {
                        if self.gesture.touch_end_at.is_some() {
                            self.gesture.on_touch_end().await;
                        }
                    }
                    Ok(Sample::None) => {}
                    Err(_) => {
                        error!("pinnacle: SPI read failed");
                        Timer::after_millis(10).await;
                    }
                },
                Either::Second(_) => {
                    let now = Instant::now();
                    if self.gesture.touch_end_at.is_some_and(|t| t <= now) {
                        self.gesture.on_touch_end().await;
                    }
                    if self.gesture.glide_at.is_some_and(|t| t <= now) {
                        self.gesture.on_glide_frame().await;
                    }
                    if self.gesture.release_at.is_some_and(|t| t <= now) {
                        self.gesture.on_release_due().await;
                    }
                }
            }
        }
    }
}

/// Touch tracking, touch key, and inertial glide. Fixed-point math in Q8
/// (value * 256), integer only, mirroring `peripheral_gesture.c`.
struct Gesture {
    touching: bool,
    confirm_count: u8,
    has_prev: bool,
    prev: (i32, i32),
    last_sample_ms: u64,
    /// Ring buffer of (dx, dy, dt_ms) for the lift speed, motion in Q8
    window: [(i32, i32, u32); VEL_WINDOW],
    window_head: usize,
    window_len: usize,
    glide_delta_q8: (i32, i32),
    glide_accum_q8: (i32, i32),
    /// Sub-unit cursor motion left over from earlier samples
    motion_accum_q8: (i32, i32),
    /// Longest step of this touch, to tell swipes from taps
    peak_step: i32,
    /// When the touch key was pressed
    pressed_at: Instant,
    /// A swipe's touch key release, held back until this time
    release_at: Option<Instant>,
    touch_end_at: Option<Instant>,
    glide_at: Option<Instant>,
}

impl Gesture {
    const fn new() -> Self {
        Self {
            touching: false,
            confirm_count: 0,
            has_prev: false,
            prev: (0, 0),
            last_sample_ms: 0,
            window: [(0, 0, 0); VEL_WINDOW],
            window_head: 0,
            window_len: 0,
            glide_delta_q8: (0, 0),
            glide_accum_q8: (0, 0),
            motion_accum_q8: (0, 0),
            peak_step: 0,
            pressed_at: Instant::from_ticks(0),
            release_at: None,
            touch_end_at: None,
            glide_at: None,
        }
    }

    async fn on_sample(&mut self, x: i32, y: i32) {
        let now = Instant::now();
        self.touch_end_at = Some(now + TOUCH_TIMEOUT);

        // First sample of a new touch: stop any glide, reset per-touch state
        if !self.touching && self.confirm_count == 0 {
            self.glide_at = None;
            self.window_head = 0;
            self.window_len = 0;
            self.glide_accum_q8 = (0, 0);
            self.motion_accum_q8 = (0, 0);
            self.peak_step = 0;
            self.has_prev = false;
            // A new touch before a held-back release: release now, so the
            // press below starts a fresh tap or hold
            if self.release_at.take().is_some() {
                publish_event_async(KeyboardEvent::key(TOUCH_KEY_ROW, TOUCH_KEY_COL, false)).await;
            }
            self.last_sample_ms = now.as_millis();
        }

        let (mut dx, mut dy) = if self.has_prev {
            let (dx, dy) = (x - self.prev.0, y - self.prev.1);
            let longest = dx.abs().max(dy.abs());
            if longest > MAX_DELTA {
                (dx * MAX_DELTA / longest, dy * MAX_DELTA / longest)
            } else {
                (dx, dy)
            }
        } else {
            (0, 0)
        };
        self.prev = (x, y);
        self.has_prev = true;
        self.peak_step = self.peak_step.max(dx.abs().max(dy.abs()));

        // Rotate, then apply the expo gain, in Q8
        let rx = dx * ROTATE_COS_Q8 - dy * ROTATE_SIN_Q8;
        let ry = dx * ROTATE_SIN_Q8 + dy * ROTATE_COS_Q8;
        let gain = expo_gain_q8(rx.abs().max(ry.abs()));
        let (qx, qy) = (rx * gain >> 8, ry * gain >> 8);
        // Whole units go out now, fractions carry to the next sample so slow
        // moves aren't lost
        let (ax, ay) = &mut self.motion_accum_q8;
        *ax += qx;
        *ay += qy;
        (dx, dy) = (*ax >> 8, *ay >> 8);
        *ax -= dx << 8;
        *ay -= dy << 8;

        let now_ms = now.as_millis();
        let dt = now_ms.saturating_sub(self.last_sample_ms).max(1) as u32;
        self.last_sample_ms = now_ms;
        // The glide continues at the speed the cursor moved, expo included
        self.window[self.window_head] = (qx, qy, dt);
        self.window_head = (self.window_head + 1) % VEL_WINDOW;
        self.window_len = (self.window_len + 1).min(VEL_WINDOW);

        if dx != 0 || dy != 0 {
            publish_motion(dx, dy).await;
        }

        if !self.touching {
            self.confirm_count += 1;
            if self.confirm_count >= TOUCH_CONFIRM_SAMPLES {
                self.touching = true;
                self.pressed_at = now;
                publish_event_async(KeyboardEvent::key(TOUCH_KEY_ROW, TOUCH_KEY_COL, true)).await;
            }
        }
    }

    async fn on_touch_end(&mut self) {
        self.touch_end_at = None;
        let was_touching = self.touching;
        self.touching = false;
        self.confirm_count = 0;
        if !was_touching {
            // Never confirmed: a phantom blip, no key was pressed
            return;
        }

        // The last sample before lift-off is skewed as the contact patch
        // shrinks, which bends the glide direction: leave it out.
        let newest = (self.window_head + VEL_WINDOW - 1) % VEL_WINDOW;
        let skip_newest = self.window_len >= 3;
        let n = self.window_len - skip_newest as usize;
        if n > 0 {
            let (sum_dx, sum_dy, sum_dt) = self.window[..self.window_len]
                .iter()
                .enumerate()
                .filter(|&(i, _)| !(skip_newest && i == newest))
                .fold((0i32, 0i32, 0u32), |(a, b, c), (_, &(dx, dy, dt))| (a + dx, b + dy, c + dt));
            // Samples stopping well before the lift (not expected while a
            // finger is down) slow the measured speed accordingly. One
            // sample interval of silence is normal before the lift packet;
            // after a timeout the silence is the timeout itself, not a hold.
            let silent = Instant::now().as_millis().saturating_sub(self.last_sample_ms);
            let silent = if silent >= TOUCH_TIMEOUT.as_millis() { ANIMATE_MS } else { silent };
            let sum_dt = (silent as u32 * n as u32).max(sum_dt).max(1);
            let fast = (sum_dx.unsigned_abs() + sum_dy.unsigned_abs()) * 10 > VELOCITY_THRESHOLD * sum_dt * 256;
            if fast {
                let scale = |sum: i32| {
                    (sum as i64 * ANIMATE_MS as i64 * SPEED_SCALE / (sum_dt as i64 * 100)) as i32
                };
                self.glide_delta_q8 = (scale(sum_dx), scale(sum_dy));
                self.glide_accum_q8 = (0, 0);
                // The first frame takes the slot of the sample that would
                // have followed the last one
                let next = Instant::from_millis(self.last_sample_ms) + Duration::from_millis(ANIMATE_MS);
                self.glide_at = Some(next.max(Instant::now()));
            }
        }

        let release_at = self.pressed_at + SWIPE_RELEASE_AFTER;
        if self.peak_step > TAP_MAX_STEP && release_at > Instant::now() {
            self.release_at = Some(release_at);
        } else {
            publish_event_async(KeyboardEvent::key(TOUCH_KEY_ROW, TOUCH_KEY_COL, false)).await;
        }
    }

    async fn on_release_due(&mut self) {
        self.release_at = None;
        publish_event_async(KeyboardEvent::key(TOUCH_KEY_ROW, TOUCH_KEY_COL, false)).await;
    }

    async fn on_glide_frame(&mut self) {
        let (dx, dy) = &mut self.glide_delta_q8;
        *dx = *dx * (1000 - DECAY_PER_MILLE) / 1000;
        *dy = *dy * (1000 - DECAY_PER_MILLE) / 1000;
        let (ax, ay) = &mut self.glide_accum_q8;
        *ax += *dx;
        *ay += *dy;
        let sx = *ax >> 8;
        let sy = *ay >> 8;
        *ax -= sx << 8;
        *ay -= sy << 8;
        if sx != 0 || sy != 0 {
            publish_motion(sx, sy).await;
        }

        // Keep going until the speed is under 0.01 unit/frame and less than
        // half a unit is left over
        let (dx, dy) = self.glide_delta_q8;
        let (ax, ay) = self.glide_accum_q8;
        self.glide_at = if dx.abs() > 2 || dy.abs() > 2 || ax.abs() > 127 || ay.abs() > 127 {
            Some(Instant::now() + Duration::from_millis(ANIMATE_MS))
        } else {
            None
        };
    }
}

/// Expo gain in Q8 for a step whose longer axis is `len_q8` (Q8 pad units)
fn expo_gain_q8(len_q8: i32) -> i32 {
    let rise = (256 - EXPO_MIN_Q8) * len_q8 / (EXPO_UNITY * 256);
    (EXPO_MIN_Q8 + rise).min(EXPO_MAX_Q8)
}

async fn publish_motion(dx: i32, dy: i32) {
    let axis = |axis, value: i32| AxisEvent {
        typ: AxisValType::Rel,
        axis,
        value: value.clamp(i16::MIN as i32, i16::MAX as i32) as i16,
    };
    publish_event_async(PointingEvent {
        device_id: POINTING_DEVICE_ID,
        axes: [axis(Axis::X, dx), axis(Axis::Y, dy), axis(Axis::Z, 0)],
    })
    .await;
}
