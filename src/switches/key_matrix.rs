use core::mem::{transmute_copy, MaybeUninit};

use cortex_m::prelude::_embedded_hal_adc_OneShot;
use embedded_hal::{adc::Channel, blocking::delay::DelayUs, digital::v2::OutputPin as _};
use rp2040_hal::{
    adc::Adc,
    gpio::{DynPinId, FunctionSioOutput, Pin, PullDown},
};
use rustkbd::{keyboard::KeySwitches, Vec};

use super::{buffer::Buffer, kalman_filter::KalmanFilter, switch_identifier::SwitchIdentifier};

pub struct KeyMatrix<
    D: DelayUs<u16>,
    P: Channel<Adc, ID = u8>,
    const ROWS: usize,
    const CSELS: usize,
    const COLS: usize,
> {
    rows: [Pin<DynPinId, FunctionSioOutput, PullDown>; ROWS],
    mux_selectors: [Pin<DynPinId, FunctionSioOutput, PullDown>; CSELS],
    mux_enabled: Pin<DynPinId, FunctionSioOutput, PullDown>,
    opa_shutdown: Pin<DynPinId, FunctionSioOutput, PullDown>,
    rst_charge: Pin<DynPinId, FunctionSioOutput, PullDown>,
    adc: Adc,
    adc_pin: P,
    delay: D,
    filters: [[KalmanFilter; COLS]; ROWS],
    buffers: [[Buffer<3>; COLS]; ROWS],
    values: [[u16; COLS]; ROWS],
}

impl<
        D: DelayUs<u16>,
        P: Channel<Adc, ID = u8>,
        const ROWS: usize,
        const CSELS: usize,
        const COLS: usize,
    > KeyMatrix<D, P, ROWS, CSELS, COLS>
{
    pub const THRESHOLD: f32 = 40.0;

    #[allow(clippy::too_many_arguments)]
    pub fn new(
        rows: [Pin<DynPinId, FunctionSioOutput, PullDown>; ROWS],
        mux_selectors: [Pin<DynPinId, FunctionSioOutput, PullDown>; CSELS],
        mut mux_enabled: Pin<DynPinId, FunctionSioOutput, PullDown>,
        mut opa_shutdown: Pin<DynPinId, FunctionSioOutput, PullDown>,
        mut rst_charge: Pin<DynPinId, FunctionSioOutput, PullDown>,
        adc: Adc,
        adc_pin: P,
        delay: D,
    ) -> KeyMatrix<D, P, ROWS, CSELS, COLS> {
        mux_enabled.set_high().ok();
        opa_shutdown.set_low().ok();
        rst_charge.set_high().ok();

        let mut filters: [[MaybeUninit<KalmanFilter>; COLS]; ROWS] =
            unsafe { MaybeUninit::uninit().assume_init() };
        for slot in filters.iter_mut() {
            for slot in slot.iter_mut() {
                *slot = MaybeUninit::new(KalmanFilter::new(2.0, 10.0));
            }
        }

        let mut buffers: [[MaybeUninit<Buffer<3>>; COLS]; ROWS] =
            unsafe { MaybeUninit::uninit().assume_init() };
        for slot in buffers.iter_mut() {
            for slot in slot.iter_mut() {
                *slot = MaybeUninit::new(Buffer::new());
            }
        }

        KeyMatrix {
            rows,
            mux_selectors,
            mux_enabled,
            opa_shutdown,
            rst_charge,
            adc,
            adc_pin,
            delay,
            filters: unsafe { transmute_copy::<_, [[KalmanFilter; COLS]; ROWS]>(&filters) },
            buffers: unsafe { transmute_copy::<_, [[Buffer<3>; COLS]; ROWS]>(&buffers) },
            values: [[0; COLS]; ROWS],
        }
    }

    pub fn values(&self) -> [[u16; COLS]; ROWS] {
        self.values
    }

    pub fn is_any_key_pressed(&self) -> bool {
        self.values
            .map(|row| row.iter().any(|v| *v as f32 > Self::THRESHOLD))
            .iter()
            .any(|r| *r)
    }
}

impl<
        D: DelayUs<u16>,
        P: Channel<Adc, ID = u8>,
        const ROWS: usize,
        const CSELS: usize,
        const COLS: usize,
    > KeySwitches<2, 12> for KeyMatrix<D, P, ROWS, CSELS, COLS>
{
    type Identifier = SwitchIdentifier;

    fn scan(&mut self) -> Vec<Self::Identifier, 12> {
        let mut keys = Vec::<Self::Identifier, 12>::new();

        // opa_shutdownとmux_enabledは実際はHi/Loが逆
        self.opa_shutdown.set_high().ok();
        self.mux_enabled.set_low().ok();

        for col in 0..COLS {
            // マルチプレクサの設定
            self.mux_enabled.set_high().ok();
            for sel in 0..CSELS {
                self.mux_selectors[sel]
                    .set_state((col & (1 << sel) != 0).into())
                    .ok();
            }
            self.mux_enabled.set_low().ok();
            self.delay.delay_us(10);

            for row in 0..ROWS {
                self.rst_charge.set_low().ok();
                self.delay.delay_us(40);
                self.rows[row].set_high().unwrap();
                self.delay.delay_us(8);

                let val: u16 = self.adc.read(&mut self.adc_pin).unwrap_or(0);
                self.delay.delay_us(8);
                // if col == 0 && row == 0 {
                //     defmt::debug!("{}", val);
                // }
                let val = self.filters[row][col].predict(val.into());
                self.values[row][col] = val as u16;
                if self.buffers[row][col].update(val > Self::THRESHOLD) {
                    let key_identifier = SwitchIdentifier {
                        row: row as u8,
                        col: col as u8,
                    };
                    keys.push(key_identifier).ok();
                }

                self.rows[row].set_low().unwrap();
                self.rst_charge.set_high().ok();
                self.delay.delay_us(5);
            }
        }

        self.mux_enabled.set_high().ok();
        self.opa_shutdown.set_low().ok();

        keys
    }
}
