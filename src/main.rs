#![no_std]
#![no_main]

use core::cell::RefCell;

use cortex_m::{delay::Delay, interrupt::Mutex};
use defmt_rtt as _;
use embedded_hal::watchdog::{Watchdog as _, WatchdogEnable};
use embedded_time::{
    duration::Extensions as _, fixed_point::FixedPoint as _, rate::Extensions as _,
};
use layout::{Layer, Layout};
use panic_probe as _;
use rp_pico::{
    entry,
    hal::{
        clocks,
        gpio::{bank0::Gpio26, FloatingInput, Pin},
        multicore::{Multicore, Stack},
        sio::Spinlock0,
        timer::{Alarm, Alarm0},
        usb::UsbBus,
        Adc, Clock, Sio, Timer, Watchdog, I2C,
    },
    pac::{interrupt, CorePeripherals, Interrupt, Peripherals, NVIC},
    Pins,
};
use rustkbd::{
    keyboard::Controller,
    usb::{DeviceInfo, UsbCommunicator},
};
use switches::KeyMatrix;
use usb_device::class_prelude::UsbBusAllocator;

use crate::drawing::Display;

mod drawing;
mod layout;
mod switches;

type KeyboardType = Controller<
    2,
    12,
    UsbCommunicator<'static, UsbBus>,
    KeyMatrix<Delay, Pin<Gpio26, FloatingInput>, 4, 4, 12>,
    Layer,
    Layout,
>;
static mut KEYBOARD: Mutex<RefCell<Option<KeyboardType>>> = Mutex::new(RefCell::new(None));
static mut ALARM: Mutex<RefCell<Option<Alarm0>>> = Mutex::new(RefCell::new(None));
static mut CORE1_STACK: Stack<4096> = Stack::new();

const USB_SEND_INTERVAL_MICROSECONDS: u32 = 10_000;

#[entry]
fn main() -> ! {
    // These variables must be static due to lifetime constraints
    static mut USB_BUS: Option<UsbBusAllocator<UsbBus>> = None;

    defmt::info!("Launching necoboard v2!");

    let mut pac = Peripherals::take().unwrap();
    let core = CorePeripherals::take().unwrap();
    // The single-cycle I/O block controls our GPIO pins
    let mut sio = Sio::new(pac.SIO);
    let pins = Pins::new(
        pac.IO_BANK0,
        pac.PADS_BANK0,
        sio.gpio_bank0,
        &mut pac.RESETS,
    );
    // Set up the watchdog driver - needed by the clock setup code
    let mut watchdog = Watchdog::new(pac.WATCHDOG);
    // The default is to generate a 125 MHz system clock
    let clocks = clocks::init_clocks_and_plls(
        rp_pico::XOSC_CRYSTAL_FREQ,
        pac.XOSC,
        pac.CLOCKS,
        pac.PLL_SYS,
        pac.PLL_USB,
        &mut pac.RESETS,
        &mut watchdog,
    )
    .ok()
    .unwrap();

    let mut timer = Timer::new(pac.TIMER, &mut pac.RESETS);
    let mut alarm = timer.alarm_0().unwrap();
    alarm
        .schedule(USB_SEND_INTERVAL_MICROSECONDS.microseconds())
        .unwrap();
    alarm.enable_interrupt();
    cortex_m::interrupt::free(|cs| unsafe {
        ALARM.borrow(cs).replace(Some(alarm));
    });

    let usb_bus = UsbBusAllocator::new(UsbBus::new(
        pac.USBCTRL_REGS,
        pac.USBCTRL_DPRAM,
        clocks.usb_clock,
        true,
        &mut pac.RESETS,
    ));
    *USB_BUS = Some(usb_bus);

    let mut mc = Multicore::new(&mut pac.PSM, &mut pac.PPB, &mut sio.fifo);
    let cores = mc.cores();
    let core1 = &mut cores[1];

    let i2c = I2C::i2c0(
        pac.I2C0,
        pins.gpio12.into_mode(),
        pins.gpio13.into_mode(),
        400_000u32.Hz(),
        &mut pac.RESETS,
        clocks.peripheral_clock.freq(),
    );
    let mut display = Display::new(i2c);

    let key_matrix = KeyMatrix::new(
        [
            pins.gpio18.into(),
            pins.gpio19.into(),
            pins.gpio20.into(),
            pins.gpio21.into(),
        ],
        [
            pins.gpio10.into(),
            pins.gpio11.into(),
            pins.gpio9.into(),
            pins.gpio8.into(),
        ],
        pins.gpio7.into(),
        pins.voltage_monitor.into(),
        pins.gpio28.into(),
        Adc::new(pac.ADC, &mut pac.RESETS),
        pins.gpio26.into_floating_input(),
        Delay::new(core.SYST, clocks.system_clock.freq().integer()),
    );

    let device_info = DeviceInfo {
        manufacturer: "necocen",
        vendor_id: 0x0c0d,
        product_id: 0x8030,
        product_name: "necoboard v2",
        serial_number: "17",
    };

    let keyboard = Controller::new(
        UsbCommunicator::new(device_info, USB_BUS.as_ref().unwrap()),
        key_matrix,
        Layout::default(),
    );
    cortex_m::interrupt::free(|cs| unsafe {
        KEYBOARD.borrow(cs).replace(Some(keyboard));
    });

    unsafe {
        // Enable the USB interrupt
        NVIC::unmask(Interrupt::USBCTRL_IRQ);
        NVIC::unmask(Interrupt::TIMER_IRQ_0);
    }

    core1
        .spawn(unsafe { &mut CORE1_STACK.mem }, move || loop {
            let state = {
                let _lock = Spinlock0::claim();
                cortex_m::interrupt::free(|cs| unsafe {
                    KEYBOARD.borrow(cs).borrow().as_ref().unwrap().get_state()
                })
            };
            display.draw(&state);
        })
        .unwrap();

    watchdog.pause_on_debug(true);
    watchdog.start(1_000_000.microseconds());

    loop {
        cortex_m::interrupt::free(|cs| unsafe {
            let _lock = Spinlock0::claim();
            KEYBOARD
                .borrow(cs)
                .borrow()
                .as_ref()
                .map(Controller::main_loop);
        });
        watchdog.feed();
    }
}

#[allow(non_snake_case)]
#[interrupt]
fn USBCTRL_IRQ() {
    cortex_m::interrupt::free(|cs| unsafe {
        let _lock = Spinlock0::claim();
        KEYBOARD
            .borrow(cs)
            .borrow_mut()
            .as_mut()
            .map(|keyboard| keyboard.communicator.poll())
    });
}

#[allow(non_snake_case)]
#[interrupt]
fn TIMER_IRQ_0() {
    cortex_m::interrupt::free(|cs| unsafe {
        let _lock = Spinlock0::claim();
        let mut alarm = ALARM.borrow(cs).borrow_mut();
        let alarm = alarm.as_mut().unwrap();
        alarm.clear_interrupt();
        alarm
            .schedule(USB_SEND_INTERVAL_MICROSECONDS.microseconds())
            .unwrap();
        alarm.enable_interrupt();
        if let Some(Err(e)) = KEYBOARD
            .borrow(cs)
            .borrow()
            .as_ref()
            .map(Controller::send_keys)
        {
            defmt::warn!("UsbError: {}", defmt::Debug2Format(&e));
        }
    });
}
