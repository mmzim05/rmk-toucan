//! USB dongle for the Toucan on a XIAO nRF52840. It bonds the keyboard's left
//! half over BLE and relays its HID reports and Vial traffic to the host.

#![no_std]
#![no_main]

use defmt::{info, unwrap};
use defmt_rtt as _;
use embassy_executor::Spawner;
use embassy_nrf::mode::Async;
use embassy_nrf::peripherals::{RNG, USBD};
use embassy_nrf::usb::Driver;
use embassy_nrf::usb::vbus_detect::HardwareVbusDetect;
use embassy_nrf::{bind_interrupts, rng, usb};
use nrf_mpsl::Flash;
use nrf_sdc::mpsl::MultiprotocolServiceLayer;
use nrf_sdc::{self as sdc, mpsl};
use rmk::ble::{SDC_PACKET_SIZE, SDC_RX_BUFFERS, SDC_TX_BUFFERS};
use rmk::config::{DeviceConfig, StorageConfig};
use rmk::dongle::{Dongle, DongleRouter};
use rmk::run_all;
use rmk::storage::new_storage_without_keymap;
use rmk::usb::UsbTransport;
use static_cell::StaticCell;

bind_interrupts!(struct Irqs {
    USBD => usb::InterruptHandler<USBD>;
    RNG => rng::InterruptHandler<RNG>;
    EGU0_SWI0 => nrf_sdc::mpsl::LowPrioInterruptHandler;
    CLOCK_POWER => nrf_sdc::mpsl::ClockInterruptHandler, usb::vbus_detect::InterruptHandler;
    RADIO => nrf_sdc::mpsl::HighPrioInterruptHandler;
    TIMER0 => nrf_sdc::mpsl::HighPrioInterruptHandler;
    RTC0 => nrf_sdc::mpsl::HighPrioInterruptHandler;
});

/// A dongle has no probe attached, so a panic lights the XIAO's red LED
/// (P0_26, active low) instead of only logging over RTT.
#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    let p0 = embassy_nrf::pac::P0;
    p0.outclr().write(|w| w.set_pin(26, true));
    p0.dirset().write(|w| w.set_pin(26, true));
    defmt::error!("{}", defmt::Display2Format(info));
    loop {
        cortex_m::asm::wfe();
    }
}

#[embassy_executor::task]
async fn mpsl_task(mpsl: &'static MultiprotocolServiceLayer<'static>) -> ! {
    mpsl.run().await
}

/// Central-only controller: the dongle scans and connects, never advertises.
fn build_sdc<'d, const N: usize>(
    p: nrf_sdc::Peripherals<'d>,
    rng: &'d mut rng::Rng<Async>,
    mpsl: &'d MultiprotocolServiceLayer,
    mem: &'d mut sdc::Mem<N>,
) -> Result<nrf_sdc::SoftdeviceController<'d>, nrf_sdc::Error> {
    sdc::Builder::new()?
        .support_scan()
        .support_central()
        .support_dle_central()
        .support_phy_update_central()
        .support_le_2m_phy()
        .default_tx_power(8)?
        // One keyboard, one link. The controller defaults to one peripheral
        // link too, which the memory below is not sized for.
        .central_count(1)?
        .peripheral_count(0)?
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
    info!("Toucan dongle");
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
        { rmk::ble::calculate_sdc_memory_size(1, 0, SDC_PACKET_SIZE, SDC_TX_BUFFERS, SDC_RX_BUFFERS) },
    >::new();
    let sdc = unwrap!(build_sdc(sdc_p, &mut rng, mpsl, &mut sdc_mem));

    let driver = Driver::new(p.USBD, Irqs, HardwareVbusDetect::new(Irqs));
    let device_config = DeviceConfig {
        vid: 0x4c4b,
        pid: 0x5444,
        manufacturer: "beekeeb",
        product_name: "Toucan Dongle",
        ..DeviceConfig::default()
    };

    // Only the keyboard bond is stored
    let storage_config = StorageConfig {
        start_addr: 0xA0000,
        num_sectors: 6,
        ..Default::default()
    };
    let flash = Flash::take(mpsl, p.NVMC);
    let mut storage = new_storage_without_keymap(flash, storage_config).await;

    // Shared by the BLE link to the keyboard and the USB host sessions
    let router = DongleRouter::new();
    let mut dongle = Dongle::new(sdc, ble_addr(), &router);
    let mut usb_transport = UsbTransport::new(driver, device_config).with_dongle_router(&router);

    run_all!(usb_transport, dongle, storage).await;
}
