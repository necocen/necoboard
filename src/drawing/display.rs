use core::ops::Deref;

use embedded_graphics::{
    image::{Image, ImageRaw},
    pixelcolor::BinaryColor,
    prelude::Point,
    primitives::{Line, PrimitiveStyle, StyledDrawable},
    Drawable,
};

use rp2040_hal::I2C;
use rp_pico::pac::i2c0::RegisterBlock;
use ssd1306::{
    mode::BufferedGraphicsMode,
    prelude::{DisplayConfig, I2CInterface},
    rotation::DisplayRotation,
    size::DisplaySize128x32,
    I2CDisplayInterface, Ssd1306,
};

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

    pub fn draw(&mut self, values: &[[u16; 12]; 4]) {
        self.display.clear();

        // cat
        let cat = self.cats[(self.frame / 5) % 4];
        let image = Image::new(&cat, Point::new(0, 0));
        image.draw(&mut self.display).ok();

        // chart
        for (i, row) in values.iter().enumerate() {
            for (j, v) in row.iter().enumerate() {
                let k = (i * 12 + j) as i32;
                let v = *v as i32 / 5 + 4;
                Line::new(Point::new(64 + k, 32), Point::new(64 + k, 32 - v))
                    .draw_styled(
                        &PrimitiveStyle::with_stroke(BinaryColor::On, 1),
                        &mut self.display,
                    )
                    .ok();
            }
        }

        self.display.flush().ok();
        self.frame += 1;
    }
}
