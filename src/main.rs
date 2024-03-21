#![no_std]
#![no_main]

use core::{
    cell::RefCell,
    sync::atomic::{AtomicBool, Ordering},
};

use cortex_m::delay::Delay;
use critical_section::Mutex;
use defmt_rtt as _;
use drawing::Display;
use fugit::{ExtU32, MicrosDurationU32, RateExtU32};
use hal::{
    adc::AdcPin,
    clocks, entry,
    gpio::{Pins, PullUp},
    multicore::{Multicore, Stack},
    pac::{interrupt, CorePeripherals, Interrupt, Peripherals, NVIC},
    sio::Spinlock0,
    timer::{Alarm, Alarm0, Alarm1, Instant},
    usb::UsbBus,
    Adc, Clock, Sio, Timer, Watchdog, I2C,
};
use layout::Layout;
use panic_probe as _;
use rp2040_hal as hal;
use rustkbd::{
    keyboard::Controller,
    usb::{DeviceInfo, UsbCommunicator},
};
use switches::KeyMatrix;
use usb_device::class_prelude::UsbBusAllocator;

mod drawing;
mod layout;
mod switches;

/// The linker will place this boot block at the start of our program image. We
/// need this to help the ROM bootloader get our code up and running.
/// Note: This boot block is not necessary when using a rp-hal based BSP
/// as the BSPs already perform this step.
#[link_section = ".boot2"]
#[used]
pub static BOOT2: [u8; 256] = rp2040_boot2::BOOT_LOADER_GENERIC_03H;

type KeyboardType =
    Controller<2, 12, UsbCommunicator<'static, UsbBus>, KeyMatrix<Delay, 4, 4, 12>, Layout>;
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
const XTAL_FREQ_HZ: u32 = 12_000_000;

static mut CORE1_STACK: Stack<4096> = Stack::new();

#[entry]
fn main() -> ! {
    // These variables must be static due to lifetime constraints
    static mut USB_BUS: Option<UsbBusAllocator<UsbBus>> = None;

    defmt::info!("Launching necoboard v2!");

    let mut pac = Peripherals::take().unwrap();
    let core = CorePeripherals::take().unwrap();
    // Set up the watchdog driver - needed by the clock setup code
    let mut watchdog = Watchdog::new(pac.WATCHDOG);
    // The default is to generate a 125 MHz system clock
    let clocks = clocks::init_clocks_and_plls(
        XTAL_FREQ_HZ,
        pac.XOSC,
        pac.CLOCKS,
        pac.PLL_SYS,
        pac.PLL_USB,
        &mut pac.RESETS,
        &mut watchdog,
    )
    .unwrap();
    // The single-cycle I/O block controls our GPIO pins
    let mut sio = Sio::new(pac.SIO);
    let pins = Pins::new(
        pac.IO_BANK0,
        pac.PADS_BANK0,
        sio.gpio_bank0,
        &mut pac.RESETS,
    );

    let mut timer = Timer::new(pac.TIMER, &mut pac.RESETS, &clocks);
    let mut alarm0 = timer.alarm_0().unwrap();
    alarm0.schedule(USB_SEND_INTERVAL).unwrap();
    alarm0.enable_interrupt();
    let mut alarm1 = timer.alarm_1().unwrap();
    alarm1.schedule(SWITCH_SCAN_INTERVAL).unwrap();
    alarm1.enable_interrupt();
    critical_section::with(|cs| unsafe {
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
            pins.gpio18.reconfigure().into_dyn_pin(),
            pins.gpio19.reconfigure().into_dyn_pin(),
            pins.gpio20.reconfigure().into_dyn_pin(),
            pins.gpio21.reconfigure().into_dyn_pin(),
        ],
        [
            pins.gpio10.reconfigure().into_dyn_pin(),
            pins.gpio11.reconfigure().into_dyn_pin(),
            pins.gpio9.reconfigure().into_dyn_pin(),
            pins.gpio8.reconfigure().into_dyn_pin(),
        ],
        pins.gpio7.reconfigure().into_dyn_pin(),
        pins.gpio29.reconfigure().into_dyn_pin(),
        pins.gpio28.reconfigure().into_dyn_pin(),
        Adc::new(pac.ADC, &mut pac.RESETS),
        AdcPin::new(pins.gpio26).unwrap(),
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
    critical_section::with(|cs| unsafe {
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
        .spawn(unsafe { &mut CORE1_STACK.mem }, move || loop {
            if SLEEP_MODE.load(Ordering::Relaxed) {
                // スリープモードに入った最初のフレームでは黒く塗る
                display.draw_sleep();
                while SLEEP_MODE.load(Ordering::Relaxed) {
                    core::hint::spin_loop()
                }
            }

            let values = {
                let _lock = Spinlock0::claim();
                critical_section::with(|cs| unsafe {
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
    critical_section::with(|cs| unsafe {
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
    critical_section::with(|cs| unsafe {
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
    critical_section::with(|cs| unsafe {
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
        if let Some(w) = WATCHDOG.borrow(cs).borrow_mut().as_mut() {
            w.feed()
        }
        SLEEP_MODE.store(sleep_mode, Ordering::Relaxed);
    });
}
