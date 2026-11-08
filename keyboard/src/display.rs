//! The left half's Sharp memory LCD (144x168, the `sharp,ls0xx` panel from
//! the ZMK config) and the status screen drawn on it.

use core::fmt::Write as _;

use embassy_time::Timer;
use embedded_graphics::mono_font::MonoTextStyle;
use embedded_graphics::mono_font::ascii::{FONT_6X10, FONT_10X20};
use embedded_graphics::pixelcolor::BinaryColor;
use embedded_graphics::prelude::*;
use embedded_graphics::primitives::{Line, PrimitiveStyle, Rectangle, RoundedRectangle};
use embedded_graphics::text::{Alignment, Text};
use embedded_hal::digital::OutputPin;
use embedded_hal_async::spi::SpiBus;
use rmk::display::{DisplayDriver, DisplayRenderer, RenderContext};
use rmk::heapless::String;
use rmk::types::battery::BatteryStatus;

const WIDTH: usize = 144;
const HEIGHT: usize = 168;
const LINE_BYTES: usize = WIDTH / 8;

// Command bits, as sent LSB first
const CMD_WRITE: u8 = 0x01;
const CMD_VCOM: u8 = 0x02;
const CMD_CLEAR: u8 = 0x04;

/// Driver for a Sharp memory-in-pixel LCD.
///
/// The SPI bus must be mode 0, LSB first, at most 1 MHz. `cs` is active high.
/// Each flush toggles VCOM, which the panel needs regularly (about once a
/// second) to avoid DC bias, so run the display processor with a render
/// interval. Only lines that changed since the last flush are sent.
pub struct SharpLcd<S, CS> {
    spi: S,
    cs: CS,
    /// Pixel bit 1 = white (reflective), 0 = black
    frame: [[u8; LINE_BYTES]; HEIGHT],
    shown: [[u8; LINE_BYTES]; HEIGHT],
    vcom: bool,
    /// Command, then per line: address, data, trailer; then a final trailer
    tx: [u8; 1 + HEIGHT * (LINE_BYTES + 2) + 1],
}

impl<S: SpiBus, CS: OutputPin> SharpLcd<S, CS> {
    pub fn new(spi: S, cs: CS) -> Self {
        Self {
            spi,
            cs,
            frame: [[0xFF; LINE_BYTES]; HEIGHT],
            shown: [[0xFF; LINE_BYTES]; HEIGHT],
            vcom: false,
            tx: [0; 1 + HEIGHT * (LINE_BYTES + 2) + 1],
        }
    }

    async fn send(&mut self, len: usize) {
        self.cs.set_high().ok();
        Timer::after_micros(3).await;
        if self.spi.write(&self.tx[..len]).await.is_err() {
            defmt::error!("display: SPI write failed");
        }
        Timer::after_micros(1).await;
        self.cs.set_low().ok();
    }

    fn vcom_bit(&mut self) -> u8 {
        self.vcom = !self.vcom;
        if self.vcom { CMD_VCOM } else { 0 }
    }
}

impl<S: SpiBus, CS: OutputPin> DisplayDriver for SharpLcd<S, CS> {
    async fn init(&mut self) {
        self.cs.set_low().ok();
        self.tx[0] = CMD_CLEAR | self.vcom_bit();
        self.tx[1] = 0;
        self.send(2).await;
        self.shown = [[0xFF; LINE_BYTES]; HEIGHT];
    }

    async fn flush(&mut self) {
        let mut len = 1;
        for (i, (line, shown)) in self.frame.iter().zip(self.shown.iter_mut()).enumerate() {
            if line == shown {
                continue;
            }
            *shown = *line;
            self.tx[len] = (i + 1) as u8;
            self.tx[len + 1..len + 1 + LINE_BYTES].copy_from_slice(line);
            self.tx[len + 1 + LINE_BYTES] = 0;
            len += LINE_BYTES + 2;
        }
        let vcom = self.vcom_bit();
        // With nothing changed this is a bare "display" command, which only
        // flips VCOM.
        self.tx[0] = if len > 1 { CMD_WRITE } else { 0 } | vcom;
        self.tx[len] = 0;
        self.send(len + 1).await;
    }
}

impl<S, CS> OriginDimensions for SharpLcd<S, CS> {
    fn size(&self) -> Size {
        Size::new(WIDTH as u32, HEIGHT as u32)
    }
}

impl<S, CS> DrawTarget for SharpLcd<S, CS> {
    type Color = BinaryColor;
    type Error = core::convert::Infallible;

    fn draw_iter<I>(&mut self, pixels: I) -> Result<(), Self::Error>
    where
        I: IntoIterator<Item = Pixel<Self::Color>>,
    {
        for Pixel(p, color) in pixels {
            if p.x < 0 || p.y < 0 || p.x as usize >= WIDTH || p.y as usize >= HEIGHT {
                continue;
            }
            let (x, y) = (p.x as usize, p.y as usize);
            let byte = &mut self.frame[y][x / 8];
            let bit = 1 << (x % 8);
            match color {
                BinaryColor::On => *byte &= !bit,
                BinaryColor::Off => *byte |= bit,
            }
        }
        Ok(())
    }

    fn clear(&mut self, color: Self::Color) -> Result<(), Self::Error> {
        let fill = if color.is_on() { 0x00 } else { 0xFF };
        self.frame = [[fill; LINE_BYTES]; HEIGHT];
        Ok(())
    }
}

/// Layer names, by keymap index.
const LAYER_NAMES: [&str; 5] = ["BASE", "PAD", "NUM", "NAV", "GAME"];

/// Left and right battery side by side on top, the active layer below.
#[derive(Default)]
pub struct StatusRenderer;

impl DisplayRenderer<BinaryColor> for StatusRenderer {
    fn render<D: DrawTarget<Color = BinaryColor>>(&mut self, ctx: &RenderContext, d: &mut D) {
        d.clear(BinaryColor::Off).ok();

        // The right half's last reading stays cached after it disconnects
        let right = if ctx.peripherals_connected[0] {
            ctx.peripheral_batteries[0].0
        } else {
            BatteryStatus::Unavailable
        };
        draw_battery(d, 0, "LEFT", ctx.battery.0);
        draw_battery(d, 72, "RIGHT", right);

        let ink = PrimitiveStyle::with_stroke(BinaryColor::On, 1);
        Line::new(Point::new(8, 76), Point::new(135, 76)).into_styled(ink).draw(d).ok();

        let small = MonoTextStyle::new(&FONT_6X10, BinaryColor::On);
        Text::with_alignment("LAYER", Point::new(72, 100), small, Alignment::Center)
            .draw(d)
            .ok();
        RoundedRectangle::with_equal_corners(
            Rectangle::new(Point::new(12, 110), Size::new(120, 40)),
            Size::new(8, 8),
        )
        .into_styled(PrimitiveStyle::with_fill(BinaryColor::On))
        .draw(d)
        .ok();
        let mut number: String<4> = String::new();
        let name = match LAYER_NAMES.get(ctx.layer as usize) {
            Some(name) => *name,
            None => {
                write!(number, "{}", ctx.layer).ok();
                number.as_str()
            }
        };
        let inverted = MonoTextStyle::new(&FONT_10X20, BinaryColor::Off);
        Text::with_alignment(name, Point::new(72, 136), inverted, Alignment::Center)
            .draw(d)
            .ok();

        let mut mode: String<16> = String::new();
        write_connection_mode(&mut mode);
        Text::with_alignment(&mode, Point::new(72, 164), small, Alignment::Center)
            .draw(d)
            .ok();
    }
}

/// Where keys go: "USB", "BT 1".."BT 5", or "DONGLE". While no host is
/// connected, the profile it is trying to reach plus "WAITING". The status is
/// read on each render (every second), so no extra event subscriber is needed.
fn write_connection_mode(out: &mut String<16>) {
    use rmk::types::ble::BleState;
    use rmk::types::connection::ConnectionType;

    let status = rmk::state::current_connection_status();
    let ble = status.ble;
    // RMK keeps the dongle bond in the slot after the BLE profiles
    let write_profile = |out: &mut String<16>| {
        if ble.profile as usize == rmk::types::constants::NUM_BLE_PROFILE {
            write!(out, "DONGLE").ok()
        } else {
            write!(out, "BT {}", ble.profile + 1).ok()
        }
    };
    match status.decide_active() {
        Some(ConnectionType::Usb) => {
            write!(out, "USB").ok();
        }
        Some(ConnectionType::Ble) => {
            write_profile(out);
        }
        None if ble.state == BleState::Advertising => {
            write_profile(out);
            write!(out, " WAITING").ok();
        }
        None => {
            write!(out, "NO HOST").ok();
        }
    }
}

/// One battery block, 72 px wide starting at `x`: label, percentage, gauge.
fn draw_battery<D: DrawTarget<Color = BinaryColor>>(d: &mut D, x: i32, label: &str, status: BatteryStatus) {
    let center = x + 36;
    let small = MonoTextStyle::new(&FONT_6X10, BinaryColor::On);
    let large = MonoTextStyle::new(&FONT_10X20, BinaryColor::On);
    Text::with_alignment(label, Point::new(center, 14), small, Alignment::Center)
        .draw(d)
        .ok();

    let level = match status {
        BatteryStatus::Available { level, .. } => level,
        BatteryStatus::Unavailable => None,
    };
    let mut text: String<5> = String::new();
    match level {
        Some(level) => write!(text, "{level}%").ok(),
        None => write!(text, "--").ok(),
    };
    Text::with_alignment(&text, Point::new(center, 40), large, Alignment::Center)
        .draw(d)
        .ok();

    // Gauge: body with a nub on the right, filled by level
    let ink = PrimitiveStyle::with_stroke(BinaryColor::On, 1);
    let fill = PrimitiveStyle::with_fill(BinaryColor::On);
    Rectangle::new(Point::new(x + 12, 50), Size::new(44, 16))
        .into_styled(ink)
        .draw(d)
        .ok();
    Rectangle::new(Point::new(x + 56, 54), Size::new(3, 8))
        .into_styled(fill)
        .draw(d)
        .ok();
    if let Some(level) = level {
        let width = 40 * level.min(100) as u32 / 100;
        Rectangle::new(Point::new(x + 14, 52), Size::new(width, 12))
            .into_styled(fill)
            .draw(d)
            .ok();
    }
}
