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

const POLL_INTERVAL: Duration = Duration::from_secs(2);

pub struct DisplayActor {
    pub relay: Addr<relay::RelayActor>,
    display: Option<Mutex<ST7789<SPIInterfaceNoCS<Spidev, Pin>, Pin>>>,
}

impl Actor for DisplayActor {
    type Context = Context<Self>;

    fn started(&mut self, ctx: &mut Self::Context) {
        self.init_display(ctx);
        self.start_poll(ctx);
    }
}

impl DisplayActor {
    pub fn new(relay: Addr<relay::RelayActor>) -> Self {
        Self {
            relay,
            display: None,
        }
    }

    fn start_poll(&mut self, ctx: &mut <Self as Actor>::Context) {
        ctx.run_interval(POLL_INTERVAL, |act, ctx| {
            let res = act.relay.send(relay::GetInputs)
                .into_actor(act)
                .then(|res, act, ctx| {
                    match res {
                        Ok(inputs) => {
                            act.draw(inputs);
                        }
                        Err(err) => {
                            println!("Something is wrong");
                        }
                    }
                    fut::ready(())
                })
                .spawn(ctx);
        });
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

    fn draw(&self, inputs: relay::Inputs) {
        if let Some(val) = self.display.as_ref() {
            let display = &mut *val.lock().unwrap();
            let text_style = TextStyle::new(Font24x32, Rgb565::WHITE);
            let inactive_style = PrimitiveStyle::with_fill(Rgb565::GREEN);
            let active_style = PrimitiveStyle::with_fill(Rgb565::RED);

            let input1_circle =
                Circle::new(Point::new(60, 60), 40)
                    .into_styled(if inputs.states[0] == 0 { inactive_style } else { active_style });

            let input1_text = Text::new("1", Point::new(50, 50))
                .into_styled(text_style);

            let input2_circle =
                Circle::new(Point::new(180, 60), 40)
                    .into_styled(if inputs.states[1] == 0 { inactive_style } else { active_style });

            let input2_text = Text::new("2", Point::new(170, 50))
                .into_styled(text_style);

            let input3_circle =
                Circle::new(Point::new(60, 180), 40)
                    .into_styled(if inputs.states[2] == 0 { inactive_style } else { active_style });

            let input3_text = Text::new("3", Point::new(50, 170))
                .into_styled(text_style);

            let input4_circle =
                Circle::new(Point::new(180, 180), 40)
                    .into_styled(if inputs.states[3] == 0 { inactive_style } else { active_style });

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

// impl Handler<relay::Inputs> for GpioActor {
//     type Result = ();
//
//     fn handle(&mut self, msg: relay::Inputs, ctx: &mut Self::Context) {
//     }
// }