//! Left half: split central with the keymap, the status display, and the
//! trackpad's pointing processor. It connects to the host through the USB
//! dongle, or directly over BLE or USB.
//!
//! This half uses RMK's Rust API rather than the `keyboard.toml` macro,
//! because the pointing processor for the right half's trackpad needs the
//! keymap, which the macro does not hand to custom processors.

#![no_std]
#![no_main]

mod display;
mod keymap;
mod pointing_mode;

use defmt::{info, unwrap};
use defmt_rtt as _;
use embassy_executor::Spawner;
use embassy_nrf::gpio::{Input, Level, Output, OutputDrive, Pull};
use embassy_nrf::interrupt::{self, InterruptExt};
use embassy_nrf::mode::Async;
use embassy_nrf::peripherals::{RNG, SPI2, USBD};
use embassy_nrf::saadc::{self, Input as _, Saadc};
use embassy_nrf::usb::Driver;
use embassy_nrf::usb::vbus_detect::HardwareVbusDetect;
use embassy_nrf::{bind_interrupts, rng, spim, usb};
use embassy_time::Duration;
use nrf_mpsl::Flash;
use nrf_sdc::mpsl::MultiprotocolServiceLayer;
use nrf_sdc::{self as sdc, mpsl};
use panic_probe as _;
use rmk::ble::{BleTransport, SDC_PACKET_SIZE, SDC_RX_BUFFERS, SDC_TX_BUFFERS};
use rmk::config::{BleBatteryConfig, DeviceConfig, PositionalConfig, RmkConfig, StorageConfig, VialConfig};
use rmk::debounce::default_debouncer::DefaultDebouncer;
use rmk::display::DisplayProcessor;
use rmk::host::HostService;
use rmk::input_device::adc::{AnalogEventType, NrfAdc};
use rmk::input_device::battery::BatteryProcessor;
use rmk::input_device::pointing::{PointingProcessor, PointingProcessorConfig};
use rmk::keyboard::Keyboard;
use rmk::matrix::Matrix;
use rmk::processor::builtin::wpm::WpmProcessor;
use rmk::split::PeripheralMatrixConfig;
use rmk::usb::UsbTransport;
use rmk::watchdog::Nrf52Watchdog;
use rmk::{KeymapData, initialize_keymap_and_storage, run_all};
use static_cell::StaticCell;

include!(concat!(env!("OUT_DIR"), "/config_generated.rs"));

bind_interrupts!(struct Irqs {
    USBD => usb::InterruptHandler<USBD>;
    SAADC => saadc::InterruptHandler;
    RNG => rng::InterruptHandler<RNG>;
    SPI2 => spim::InterruptHandler<SPI2>;
    EGU0_SWI0 => nrf_sdc::mpsl::LowPrioInterruptHandler;
    CLOCK_POWER => nrf_sdc::mpsl::ClockInterruptHandler, usb::vbus_detect::InterruptHandler;
    RADIO => nrf_sdc::mpsl::HighPrioInterruptHandler;
    TIMER0 => nrf_sdc::mpsl::HighPrioInterruptHandler;
    RTC0 => nrf_sdc::mpsl::HighPrioInterruptHandler;
});

#[embassy_executor::task]
async fn mpsl_task(mpsl: &'static MultiprotocolServiceLayer<'static>) -> ! {
    mpsl.run().await
}

/// Hold the NAV-layer G position (row 1, col 5) to unlock Vial, in place of
/// ZMK's &studio_unlock on that key.
const UNLOCK_KEYS: &[(u8, u8)] = &[(1, 5)];

fn build_sdc<'d, const N: usize>(
    p: nrf_sdc::Peripherals<'d>,
    rng: &'d mut rng::Rng<Async>,
    mpsl: &'d MultiprotocolServiceLayer,
    mem: &'d mut sdc::Mem<N>,
) -> Result<nrf_sdc::SoftdeviceController<'d>, nrf_sdc::Error> {
    sdc::Builder::new()?
        .support_central()
        .support_adv()
        .support_peripheral()
        .support_dle_peripheral()
        .support_dle_central()
        .support_phy_update_central()
        .support_phy_update_peripheral()
        .support_le_2m_phy()
        // ZMK used CONFIG_BT_CTLR_TX_PWR_PLUS_8
        .default_tx_power(8)?
        .central_count(1)?
        .peripheral_count(rmk::ble::NUM_HOST_LINKS as u8)?
        .buffer_cfg(
            SDC_PACKET_SIZE as u16,
            SDC_PACKET_SIZE as u16,
            SDC_TX_BUFFERS,
            SDC_RX_BUFFERS,
        )?
        .build(p, rng, mpsl, mem)
}

fn ble_addr() -> [u8; 6] {
    let ficr = embassy_nrf::pac::FICR;
    let high = u64::from(ficr.deviceid(1).read());
    let addr = high << 32 | u64::from(ficr.deviceid(0).read());
    let addr = addr | 0x0000_c000_0000_0000;
    unwrap!(addr.to_le_bytes()[..6].try_into())
}

#[embassy_executor::main]
async fn main(spawner: Spawner) {
    info!("Toucan left half (central)");
    let p = embassy_nrf::init(Default::default());

    let mpsl_p = mpsl::Peripherals::new(p.RTC0, p.TIMER0, p.TEMP, p.PPI_CH19, p.PPI_CH30, p.PPI_CH31);
    let lfclk_cfg = mpsl::raw::mpsl_clock_lfclk_cfg_t {
        source: mpsl::raw::MPSL_CLOCK_LF_SRC_RC as u8,
        rc_ctiv: mpsl::raw::MPSL_RECOMMENDED_RC_CTIV as u8,
        rc_temp_ctiv: mpsl::raw::MPSL_RECOMMENDED_RC_TEMP_CTIV as u8,
        accuracy_ppm: 500,
        skip_wait_lfclk_started: mpsl::raw::MPSL_DEFAULT_SKIP_WAIT_LFCLK_STARTED != 0,
    };
    static MPSL: StaticCell<MultiprotocolServiceLayer> = StaticCell::new();
    static SESSION_MEM: StaticCell<mpsl::SessionMem<1>> = StaticCell::new();
    let mpsl = MPSL.init(unwrap!(mpsl::MultiprotocolServiceLayer::with_timeslots(
        mpsl_p,
        Irqs,
        lfclk_cfg,
        SESSION_MEM.init(mpsl::SessionMem::new())
    )));
    spawner.spawn(unwrap!(mpsl_task(&*mpsl)));
    let sdc_p = sdc::Peripherals::new(
        p.PPI_CH17, p.PPI_CH18, p.PPI_CH20, p.PPI_CH21, p.PPI_CH22, p.PPI_CH23, p.PPI_CH24, p.PPI_CH25, p.PPI_CH26,
        p.PPI_CH27, p.PPI_CH28, p.PPI_CH29,
    );
    let mut rng = rng::Rng::new(p.RNG, Irqs);
    let mut sdc_mem = sdc::Mem::<
        {
            rmk::ble::calculate_sdc_memory_size(
                1,
                rmk::ble::NUM_HOST_LINKS,
                SDC_PACKET_SIZE,
                SDC_TX_BUFFERS,
                SDC_RX_BUFFERS,
            )
        },
    >::new();
    let sdc = unwrap!(build_sdc(sdc_p, &mut rng, mpsl, &mut sdc_mem));

    let driver = Driver::new(p.USBD, Irqs, HardwareVbusDetect::new(Irqs));
    let flash = Flash::take(mpsl, p.NVMC);

    // Matrix: rows are inputs, cols are driven (col2row diodes)
    let rows = [
        Input::new(p.P0_19, Pull::Down),
        Input::new(p.P0_28, Pull::Down),
        Input::new(p.P1_01, Pull::Down),
        Input::new(p.P0_29, Pull::Down),
    ];
    let cols = [
        Output::new(p.P0_09, Level::Low, OutputDrive::Standard),
        Output::new(p.P0_04, Level::Low, OutputDrive::Standard),
        Output::new(p.P0_10, Level::Low, OutputDrive::Standard),
        Output::new(p.P0_05, Level::Low, OutputDrive::Standard),
        Output::new(p.P1_11, Level::Low, OutputDrive::Standard),
        Output::new(p.P1_12, Level::Low, OutputDrive::Standard),
    ];

    // Battery: XIAO's VBAT divider (1M / 510k) on P0_31, switched on by
    // pulling P0_14 low. Kept low for as long as the firmware runs.
    let _vbat_divider = Output::new(p.P0_14, Level::Low, OutputDrive::Standard);
    let mut channel = saadc::ChannelConfig::single_ended(p.P0_31.degrade_saadc());
    // The divider's ~340k source impedance needs the longest acquisition time
    // (ZMK's XIAO driver uses 40us too); the 10us default reads low and jumpy.
    channel.time = saadc::Time::_40US;
    interrupt::SAADC.set_priority(interrupt::Priority::P3);
    let saadc = Saadc::new(p.SAADC, Irqs, saadc::Config::default(), [channel]);
    saadc.calibrate().await;
    let mut battery_adc = NrfAdc::new(saadc, [AnalogEventType::Battery], [0], Duration::from_secs(12), None);
    let mut battery = const { BatteryProcessor::new(510, 1510) };

    let device_config = DeviceConfig {
        vid: 0x4c4b,
        pid: 0x5443,
        manufacturer: "beekeeb",
        product_name: "Toucan",
        ..DeviceConfig::default()
    };
    let storage_config = StorageConfig {
        start_addr: 0xA0000,
        num_sectors: 32,
        ..Default::default()
    };
    let rmk_config = RmkConfig {
        device_config,
        vial_config: VialConfig::new(VIAL_KEYBOARD_ID, VIAL_KEYBOARD_DEF, UNLOCK_KEYS),
        ble_battery_config: BleBatteryConfig::default(),
        storage_config,
    };

    let mut keymap_data = KeymapData::new(keymap::default_keymap());
    let mut behavior_config = keymap::behavior_config();
    let positional_config = PositionalConfig::default();
    let (keymap, mut storage) = initialize_keymap_and_storage(
        &mut keymap_data,
        flash,
        &storage_config,
        &mut behavior_config,
        &positional_config,
    )
    .await;

    let mut matrix = Matrix::<_, _, _, 4, 6, true>::new(rows, cols, DefaultDebouncer::new());
    let mut keyboard = Keyboard::new(&keymap);
    let host_service = HostService::new(&keymap, &rmk_config);

    // Status display: Sharp LCD on SCK P1_13, MOSI P1_15, CS P0_03 (active
    // high). Redrawing every second keeps the panel's VCOM toggling.
    let mut spi_config = spim::Config::default();
    spi_config.frequency = spim::Frequency::M1;
    spi_config.mode = spim::MODE_0;
    spi_config.bit_order = spim::BitOrder::LsbFirst;
    let spi = spim::Spim::new_txonly(p.SPI2, Irqs, p.P1_13, p.P1_15, spi_config);
    let cs = Output::new(p.P0_03, Level::Low, OutputDrive::Standard);
    let mut display = DisplayProcessor::with_renderer(display::SharpLcd::new(spi, cs), display::StatusRenderer)
        .with_render_interval(Duration::from_secs(1));

    // Trackpad motion arrives from the right half; turn it into mouse reports
    let mut trackpad = PointingProcessor::new(&keymap, PointingProcessorConfig::default());
    trackpad.set_pointing_mode(pointing_mode::CURSOR);
    let mut pointing_mode = pointing_mode::PointingModeByLayer;

    let mut wpm = WpmProcessor::new();
    let mut usb_transport = UsbTransport::new(driver, rmk_config.device_config.clone()).with_host_service(&host_service);
    let mut ble_transport = BleTransport::new(
        sdc,
        ble_addr(),
        rmk_config,
        [PeripheralMatrixConfig {
            rows: 4,
            cols: 6,
            row_offset: 0,
            col_offset: 6,
        }],
    )
    .with_host_service(&host_service);
    let mut watchdog = Nrf52Watchdog::default_runner(p.WDT);

    run_all!(
        matrix,
        storage,
        keyboard,
        battery_adc,
        battery,
        display,
        trackpad,
        pointing_mode,
        wpm,
        usb_transport,
        ble_transport,
        watchdog
    )
    .await;
}
