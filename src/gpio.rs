use std::time::Duration;

use actix::prelude::*;
use linux_embedded_hal::Pin;

use crate::relay;

const OPTOCOUPLE_PIN_GPIO_NO: u64 = 7;
const POLL_INTERVAL: Duration = Duration::from_secs(2);

pub struct GpioActor {
    pub relay: Addr<relay::RelayActor>,
}

impl Actor for GpioActor {
    type Context = Context<Self>;

    fn started(&mut self, ctx: &mut Self::Context) {
        self.start_poll(ctx);
    }
}

impl GpioActor {
    pub fn new(relay: Addr<relay::RelayActor>) -> Self {
        Self {
            relay,
        }
    }

    fn start_poll(&self, ctx: &mut <Self as Actor>::Context) {
        ctx.run_interval(POLL_INTERVAL, |act, ctx| {
            let res = act.relay.send(relay::GetInputs)
                .into_actor(act)
                .then(|res, _, ctx| {
                    match res {
                        Ok(inputs) => {
                            let opto_pin = Pin::new(OPTOCOUPLE_PIN_GPIO_NO);

                            match inputs.states.iter().find(|&&input_state| input_state == 1) {
                                Some(_) => opto_pin.set_value(1).unwrap(),
                                None => opto_pin.set_value(0).unwrap(),
                            }
                        }
                        Err(err) => {
                            println!("Something is wrong");
                        },
                    }
                    fut::ready(())
                })
                .spawn(ctx);
        });
    }
}

// impl Handler<relay::Inputs> for GpioActor {
//     type Result = ();
//
//     fn handle(&mut self, msg: relay::Inputs, ctx: &mut Self::Context) {
//     }
// }