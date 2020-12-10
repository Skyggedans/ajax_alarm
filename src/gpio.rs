use std::time::Duration;

use actix::prelude::*;
use linux_embedded_hal::Pin;

use crate::relay;
use crate::relay::{RelayStatus, RelayActor, RegisterForStatus, UnregisterForStatus};

const OPTOCOUPLE_PIN_GPIO_NO: u64 = 7;


pub struct GpioActor {
    id: usize,
}

impl Actor for GpioActor {
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
    }

    fn stopping(&mut self, _: &mut Self::Context) -> Running {
        RelayActor::from_registry().do_send(UnregisterForStatus(self.id));

        Running::Stop
    }
}

impl GpioActor {
    pub fn new() -> Self {
        Self {
            id: 0,
        }
    }

    fn set_pins(&self, inputs: Vec<u32>) {
        let opto_pin = Pin::new(OPTOCOUPLE_PIN_GPIO_NO);

        match inputs.iter().find(|&&input_state| input_state == 1) {
            Some(_) => opto_pin.set_value(1).unwrap(),
            None => opto_pin.set_value(0).unwrap(),
        }
    }
}


impl Handler<RelayStatus> for GpioActor {
    type Result = ();

    fn handle(&mut self, message: RelayStatus, ctx: &mut Self::Context) -> Self::Result {
        self.set_pins(message.inputs);
    }
}