#![no_main]
#![no_std]

mod trackpad;

use rmk::macros::rmk_peripheral;

// The peripheral macro has no `add_interrupt!` hook, so the trackpad's SPI
// interrupt gets its own binding.
::embassy_nrf::bind_interrupts!(struct SpiIrqs {
    SPI2 => ::embassy_nrf::spim::InterruptHandler<::embassy_nrf::peripherals::SPI2>;
});

/// The battery ADC generated from `[split.peripheral.battery]` samples with
/// the SAADC's 10us default, which reads the XIAO's high-impedance VBAT
/// divider low. This raises its channel to 40us once everything is set up
/// (only the very first reading still uses 10us), then idles.
pub struct BatteryAdcAcqTime;

impl rmk::core_traits::Runnable for BatteryAdcAcqTime {
    async fn run(&mut self) -> ! {
        use embassy_nrf::pac;
        pac::SAADC
            .ch(0)
            .config()
            .modify(|w| w.set_tacq(pac::saadc::vals::Tacq::_40us));
        core::future::pending().await
    }
}

#[rmk_peripheral(id = 0)]
mod keyboard_peripheral {
    #[register_processor]
    fn battery_adc_acq_time() -> crate::BatteryAdcAcqTime {
        crate::BatteryAdcAcqTime
    }

    /// Cirque Pinnacle on SPI: SCK P1_13, MISO P1_14, MOSI P1_15, CS P0_03
    /// (active low), data ready on P0_02.
    #[register_processor]
    fn trackpad() -> crate::trackpad::Trackpad<
        ::embassy_nrf::spim::Spim<'static>,
        ::embassy_nrf::gpio::Output<'static>,
        ::embassy_nrf::gpio::Input<'static>,
    > {
        use ::embassy_nrf::gpio::{Input, Level, Output, OutputDrive, Pull};
        use ::embassy_nrf::spim::{self, Spim};

        let mut config = spim::Config::default();
        config.frequency = spim::Frequency::M1;
        config.mode = spim::MODE_1;
        let spi = Spim::new(p.SPI2, crate::SpiIrqs, p.P1_13, p.P1_14, p.P1_15, config);
        let cs = Output::new(p.P0_03, Level::High, OutputDrive::Standard);
        let dr = Input::new(p.P0_02, Pull::None);
        crate::trackpad::Trackpad::new(spi, cs, dr)
    }
}
