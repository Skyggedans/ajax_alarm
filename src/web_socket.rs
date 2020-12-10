use std::sync::Mutex;
use std::time::{Duration, Instant};

use actix::prelude::*;
use actix_web::{
    App, Error, HttpRequest, HttpResponse, HttpServer, Responder, web,
};
use actix_web_actors::ws;
use actix_web_actors::ws::WebsocketContext;
use futures_util::task::SpawnExt;
use serde_json::json;

use crate::relay;
use crate::relay::{RegisterForStatus, RelayActor, RelayStatus, UnregisterForStatus};

const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(2);
const CLIENT_TIMEOUT: Duration = Duration::from_secs(10);


pub struct ClientWebSocket {
    pub id: usize,
    pub hb: Instant,
}

impl Actor for ClientWebSocket {
    type Context = ws::WebsocketContext<Self>;

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

        self.hb(ctx);
    }

    fn stopping(&mut self, _: &mut Self::Context) -> Running {
        RelayActor::from_registry().do_send(UnregisterForStatus(self.id));

        Running::Stop
    }
}

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
    pub fn new() -> Self {
        Self {
            id: 0,
            hb: Instant::now(),
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
        });
    }
}

impl Handler<RelayStatus> for ClientWebSocket {
    type Result = ();

    fn handle(&mut self, message: RelayStatus, ctx: &mut Self::Context) -> Self::Result {
        ctx.text(json!(message).to_string());
    }
}