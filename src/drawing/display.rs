use core::ops::Deref;

use embedded_graphics::{
    image::{Image, ImageRaw},
    mono_font::{ascii::FONT_6X10, MonoTextStyle},
    pixelcolor::BinaryColor,
    prelude::Point,
    text::Text,
    Drawable,
};
use heapless::String;
use rp2040_hal::I2C;
use rp_pico::pac::i2c0::RegisterBlock;
use rustkbd::keyboard::KeyboardState;
use ssd1306::{
    mode::BufferedGraphicsMode,
    prelude::{DisplayConfig, I2CInterface},
    rotation::DisplayRotation,
    size::DisplaySize128x32,
    I2CDisplayInterface, Ssd1306,
};

use crate::layout::Layer;

pub struct Display<I: Deref<Target = RegisterBlock>, J> {
    cats: [ImageRaw<'static, BinaryColor>; 4],
    display: Ssd1306<
        I2CInterface<I2C<I, J>>,
        DisplaySize128x32,
        BufferedGraphicsMode<DisplaySize128x32>,
    >,
    frame: usize,
}

impl<I: Deref<Target = RegisterBlock>, J> Display<I, J> {
    pub fn new(i2c: I2C<I, J>) -> Display<I, J> {
        let interface = I2CDisplayInterface::new(i2c);
        let mut display = Ssd1306::new(interface, DisplaySize128x32, DisplayRotation::Rotate0)
            .into_buffered_graphics_mode();
        display.init().ok();
        let cat0 = ImageRaw::new(include_bytes!("./cat/cat0.raw"), 64);
        let cat1 = ImageRaw::new(include_bytes!("./cat/cat1.raw"), 64);
        let cat2 = ImageRaw::new(include_bytes!("./cat/cat2.raw"), 64);
        let cat3 = ImageRaw::new(include_bytes!("./cat/cat3.raw"), 64);
        Display {
            cats: [cat0, cat1, cat2, cat3],
            display,
            frame: 0,
        }
    }

    pub fn draw<const RO: usize>(&mut self, state: &KeyboardState<Layer, RO>) {
        let char_style = MonoTextStyle::new(&FONT_6X10, BinaryColor::On);
        self.display.clear();

        // cat
        let cat = self.cats[(self.frame / 5) % 4];
        let image = Image::new(&cat, Point::new(0, 0));
        image.draw(&mut self.display).ok();

        // print pressed keys
        let mut string = String::<6>::new();
        state
            .keys
            .iter()
            .filter(|key| key.is_keyboard_key())
            .cloned()
            .map(From::from)
            .for_each(|c| {
                string.push(c).ok();
            });
        Text::new(string.as_str(), Point::new(64, 10), char_style)
            .draw(&mut self.display)
            .ok();

        self.display.flush().ok();
        self.frame += 1;
    }
}
