use std::sync::Mutex;
use std::time::{Duration, Instant};

use actix::prelude::*;
use actix_web::{
    App, Error, HttpRequest, HttpResponse, HttpServer, Responder, web,
};
use actix_web_actors::ws;
use actix_web_actors::ws::WebsocketContext;
use serde_json::json;

use crate::relay;

const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(2);
const CLIENT_TIMEOUT: Duration = Duration::from_secs(10);

pub struct ClientWebSocket {
    pub hb: Instant,
    pub relay: Addr<relay::RelayActor>,
}

impl Actor for ClientWebSocket {
    type Context = ws::WebsocketContext<Self>;

    fn started(&mut self, ctx: &mut Self::Context) {
        self.hb(ctx);
    }
}

// impl Handler<RelayStatus> for ClientWebSocket {
//     type Result = ();
//
//     fn handle(&mut self, msg: RelayStatus, ctx: &mut Self::Context) {
//         ctx.text(msg.0);
//     }
// }

impl StreamHandler<Result<ws::Message, ws::ProtocolError>> for ClientWebSocket {
    fn handle(&mut self, msg: Result<ws::Message, ws::ProtocolError>, ctx: &mut Self::Context) {
        println!("WS: {:?}", msg);

        match msg {
            Ok(ws::Message::Ping(msg)) => {
                self.hb = Instant::now();
                ctx.pong(&msg);
            }
            Ok(ws::Message::Pong(_)) => {
                self.hb = Instant::now();
            }
            Ok(ws::Message::Text(text)) => ctx.text(text),
            Ok(ws::Message::Binary(bin)) => ctx.binary(bin),
            Ok(ws::Message::Close(reason)) => {
                ctx.close(reason);
                ctx.stop();
            }
            _ => ctx.stop(),
        }
    }
}

impl ClientWebSocket {
    pub fn new(relay: Addr<relay::RelayActor>) -> Self {
        Self {
            hb: Instant::now(),
            relay,
        }
    }

    fn hb(&self, ctx: &mut <Self as Actor>::Context) {
        ctx.run_interval(HEARTBEAT_INTERVAL, |act, ctx| {
            if Instant::now().duration_since(act.hb) > CLIENT_TIMEOUT {
                println!("Websocket Client heartbeat failed, disconnecting!");
                ctx.stop();

                return;
            }

            ctx.ping(b"");

            let res = act.relay.send(relay::GetInputs)
                .into_actor(act)
                .then(|res, _, ctx| {
                    match res {
                        Ok(inputs) => {
                            ctx.text(json!(inputs).to_string());
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
