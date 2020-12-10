use std::borrow::BorrowMut;
use std::sync::Mutex;
use std::time::Duration;

use actix::prelude::*;
use display_interface_spi::SPIInterfaceNoCS;
use embedded_graphics::fonts::{Font24x32, Text};
use embedded_graphics::pixelcolor::Rgb565;
use embedded_graphics::prelude::*;
use embedded_graphics::primitives::*;
use embedded_graphics::style::*;
use linux_embedded_hal::{Delay, Pin, Spidev};
use linux_embedded_hal::spidev::{SPI_MODE_3, SpidevOptions};
use st7789::{Orientation, ST7789};

use crate::relay;
use crate::relay::{RelayStatus, RelayActor, RegisterForStatus, UnregisterForStatus};


pub struct DisplayActor {
    pub id: usize,
    display: Option<Mutex<ST7789<SPIInterfaceNoCS<Spidev, Pin>, Pin>>>,
}

impl Default for DisplayActor {
    fn default() -> DisplayActor {
        Self {
            id: 0,
            display: None,
        }
    }
}

impl Actor for DisplayActor {
    type Context = Context<Self>;

    fn started(&mut self, ctx: &mut Self::Context) {
        RelayActor::from_registry().send(RegisterForStatus(ctx.address().recipient()))
            .into_actor(self)
            .then(|res, act, ctx| {
                match res {
                    Ok(res) => act.id = res,
                    _ => ctx.stop(),
                }

                fut::ready(())
            })
            .wait(ctx);

        self.init_display(ctx);
    }

    fn stopping(&mut self, _: &mut Self::Context) -> Running {
        RelayActor::from_registry().do_send(UnregisterForStatus(self.id));

        Running::Stop
    }
}

impl DisplayActor {
    pub fn new() -> Self {
        Self::default()
    }

    fn init_display(&mut self, ctx: &mut <Self as Actor>::Context) {
        let mut delay = Delay;
        let mut spi = Spidev::open("/dev/spidev1.0").expect("error initializing SPI");

        let options = SpidevOptions::new()
            .bits_per_word(8)
            .max_speed_hz(40_000_000)
            .mode(SPI_MODE_3)
            .build();

        spi.configure(&options).expect("Error configuring SPI");

        let rst = Pin::new(1);
        let dc = Pin::new(0);
        let di = SPIInterfaceNoCS::new(spi, dc);
        let mut display = ST7789::new(di, rst, 240, 240);

        display.init(&mut delay).unwrap();
        display.set_orientation(Orientation::Portrait).unwrap();
        display.clear(Rgb565::BLACK).unwrap();
        self.display = Some(Mutex::new(display));
    }

    fn draw_status(&mut self, connected: bool, inputs: Vec<u32>) {
        if let Some(ref mut val) = self.display {
            let display = &mut *val.lock().unwrap();
            let text_style = TextStyle::new(Font24x32, Rgb565::WHITE);
            let inactive_style = PrimitiveStyle::with_fill(Rgb565::GREEN);
            let active_style = PrimitiveStyle::with_fill(Rgb565::RED);

            let input1_circle =
                Circle::new(Point::new(60, 60), 40)
                    .into_styled(if inputs[0] == 0 { inactive_style } else { active_style });

            let input1_text = Text::new("1", Point::new(50, 50))
                .into_styled(text_style);

            let input2_circle =
                Circle::new(Point::new(180, 60), 40)
                    .into_styled(if inputs[1] == 0 { inactive_style } else { active_style });

            let input2_text = Text::new("2", Point::new(170, 50))
                .into_styled(text_style);

            let input3_circle =
                Circle::new(Point::new(60, 180), 40)
                    .into_styled(if inputs[2] == 0 { inactive_style } else { active_style });

            let input3_text = Text::new("3", Point::new(50, 170))
                .into_styled(text_style);

            let input4_circle =
                Circle::new(Point::new(180, 180), 40)
                    .into_styled(if inputs[3] == 0 { inactive_style } else { active_style });

            let input4_text = Text::new("4", Point::new(170, 170))
                .into_styled(text_style);

            input1_circle.draw(display).unwrap();
            input2_circle.draw(display).unwrap();
            input3_circle.draw(display).unwrap();
            input4_circle.draw(display).unwrap();
            input1_text.draw(display).unwrap();
            input2_text.draw(display).unwrap();
            input3_text.draw(display).unwrap();
            input4_text.draw(display).unwrap();
        }
    }
}

impl Handler<RelayStatus> for DisplayActor {
    type Result = ();

    fn handle(&mut self, message: RelayStatus, ctx: &mut Self::Context) -> Self::Result {
        self.draw_status(message.connected, message.inputs);
    }
}