#![no_std]
#![no_main]

use core::{
    cell::RefCell,
    sync::atomic::{AtomicBool, Ordering},
};

use cortex_m::{delay::Delay, interrupt::Mutex};
use defmt_rtt as _;
use embedded_hal::watchdog::{Watchdog as _, WatchdogEnable};
use fugit::{ExtU32, MicrosDurationU32, RateExtU32};
use layout::Layout;
use panic_probe as _;
use rp2040_hal::{
    adc::AdcPin,
    gpio::{FunctionNull, PullDown, PullUp},
    timer::Instant,
};
use rp_pico::{
    entry,
    hal::{
        clocks,
        gpio::{bank0::Gpio26, Pin},
        multicore::{Multicore, Stack},
        sio::Spinlock0,
        timer::{Alarm, Alarm0, Alarm1},
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
    KeyMatrix<Delay, AdcPin<Pin<Gpio26, FunctionNull, PullDown>>, 4, 4, 12>,
    Layout,
>;
static mut KEYBOARD: Mutex<RefCell<Option<KeyboardType>>> = Mutex::new(RefCell::new(None));
static mut ALARM0: Mutex<RefCell<Option<Alarm0>>> = Mutex::new(RefCell::new(None));
static mut ALARM1: Mutex<RefCell<Option<Alarm1>>> = Mutex::new(RefCell::new(None));
static mut WATCHDOG: Mutex<RefCell<Option<Watchdog>>> = Mutex::new(RefCell::new(None));
static mut TIMER: Mutex<RefCell<Option<Timer>>> = Mutex::new(RefCell::new(None));
static SLEEP_MODE: AtomicBool = AtomicBool::new(false);
// 最後に何らかのキーがオンだった時のカウンタ
static mut LAST_KEYS_ON: Mutex<RefCell<Instant>> = Mutex::new(RefCell::new(Instant::from_ticks(0)));

const USB_SEND_INTERVAL: MicrosDurationU32 = MicrosDurationU32::millis(10);
const SWITCH_SCAN_INTERVAL: MicrosDurationU32 = MicrosDurationU32::millis(5);
const SLEEP_MODE_INTERVAL: MicrosDurationU32 = MicrosDurationU32::secs(10);

#[entry]
fn main() -> ! {
    // These variables must be static due to lifetime constraints
    static mut USB_BUS: Option<UsbBusAllocator<UsbBus>> = None;
    static mut CORE1_STACK: Stack<4096> = Stack::new();

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

    let mut timer = Timer::new(pac.TIMER, &mut pac.RESETS, &clocks);
    let mut alarm0 = timer.alarm_0().unwrap();
    alarm0.schedule(USB_SEND_INTERVAL).unwrap();
    alarm0.enable_interrupt();
    let mut alarm1 = timer.alarm_1().unwrap();
    alarm1.schedule(SWITCH_SCAN_INTERVAL).unwrap();
    alarm1.enable_interrupt();
    cortex_m::interrupt::free(|cs| unsafe {
        LAST_KEYS_ON.borrow(cs).replace(timer.get_counter());
        ALARM0.borrow(cs).replace(Some(alarm0));
        ALARM1.borrow(cs).replace(Some(alarm1));
        TIMER.borrow(cs).replace(Some(timer));
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
        pins.gpio12.into_function().into_pull_type::<PullUp>(),
        pins.gpio13.into_function().into_pull_type::<PullUp>(),
        400.kHz(),
        &mut pac.RESETS,
        clocks.peripheral_clock.freq(),
    );
    let mut display = Display::new(i2c);

    let key_matrix = KeyMatrix::new(
        [
            pins.gpio18.into_push_pull_output().into_dyn_pin(),
            pins.gpio19.into_push_pull_output().into_dyn_pin(),
            pins.gpio20.into_push_pull_output().into_dyn_pin(),
            pins.gpio21.into_push_pull_output().into_dyn_pin(),
        ],
        [
            pins.gpio10.into_push_pull_output().into_dyn_pin(),
            pins.gpio11.into_push_pull_output().into_dyn_pin(),
            pins.gpio9.into_push_pull_output().into_dyn_pin(),
            pins.gpio8.into_push_pull_output().into_dyn_pin(),
        ],
        pins.gpio7.into_push_pull_output().into_dyn_pin(),
        pins.voltage_monitor.into_push_pull_output().into_dyn_pin(),
        pins.gpio28.into_push_pull_output().into_dyn_pin(),
        Adc::new(pac.ADC, &mut pac.RESETS),
        AdcPin::new(pins.gpio26),
        Delay::new(core.SYST, clocks.system_clock.freq().to_Hz()),
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

    watchdog.pause_on_debug(true);
    watchdog.start(1.secs());
    cortex_m::interrupt::free(|cs| unsafe {
        KEYBOARD.borrow(cs).replace(Some(keyboard));
        WATCHDOG.borrow(cs).replace(Some(watchdog));
    });

    unsafe {
        // Enable the USB interrupt
        NVIC::unmask(Interrupt::USBCTRL_IRQ);
        NVIC::unmask(Interrupt::TIMER_IRQ_0);
        NVIC::unmask(Interrupt::TIMER_IRQ_1);
    }

    core1
        .spawn(&mut CORE1_STACK.mem, move || loop {
            if SLEEP_MODE.load(Ordering::Relaxed) {
                // スリープモードに入った最初のフレームでは黒く塗る
                display.draw_sleep();
                while SLEEP_MODE.load(Ordering::Relaxed) {
                    core::hint::spin_loop()
                }
            }

            let values = {
                let _lock = Spinlock0::claim();
                cortex_m::interrupt::free(|cs| unsafe {
                    KEYBOARD
                        .borrow(cs)
                        .borrow()
                        .as_ref()
                        .unwrap()
                        .key_switches
                        .values()
                })
            };
            display.draw(&values);
        })
        .unwrap();

    loop {
        cortex_m::asm::wfi();
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
        let mut alarm = ALARM0.borrow(cs).borrow_mut();
        let alarm = alarm.as_mut().unwrap();
        alarm.clear_interrupt();
        alarm.schedule(USB_SEND_INTERVAL).unwrap();
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

#[allow(non_snake_case)]
#[interrupt]
fn TIMER_IRQ_1() {
    cortex_m::interrupt::free(|cs| unsafe {
        let _lock = Spinlock0::claim();
        let mut alarm = ALARM1.borrow(cs).borrow_mut();
        let alarm = alarm.as_mut().unwrap();
        alarm.clear_interrupt();

        let mut keyboard = KEYBOARD.borrow(cs).borrow_mut();
        let keyboard = keyboard.as_mut().unwrap();
        keyboard.main_loop();

        let counter = TIMER.borrow(cs).borrow().as_ref().unwrap().get_counter();
        let mut last_counter = LAST_KEYS_ON.borrow(cs).borrow_mut();
        let should_sleep = (counter - *last_counter) >= SLEEP_MODE_INTERVAL;

        let mut sleep_mode = SLEEP_MODE.load(Ordering::Relaxed);
        if keyboard.key_switches.is_any_key_pressed() {
            *last_counter = counter;
            if sleep_mode {
                defmt::info!("Woke up!");
                sleep_mode = false;
            }
        } else if should_sleep && !sleep_mode {
            defmt::info!("Going to sleep...");
            sleep_mode = true;
        }

        alarm.schedule(SWITCH_SCAN_INTERVAL).unwrap();
        alarm.enable_interrupt();
        WATCHDOG
            .borrow(cs)
            .borrow_mut()
            .as_mut()
            .map(Watchdog::feed);
        SLEEP_MODE.store(sleep_mode, Ordering::Relaxed);
    });
}
