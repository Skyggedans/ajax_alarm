use std::sync::Mutex;
use std::time::{Duration, Instant};

use actix::actors::resolver::{Connect, Resolver};
use actix::io::{FramedWrite, WriteHandler};
use actix::prelude::*;
use actix::registry::SystemService;
use actix_web::web;
use tokio::io::{split, WriteHalf};
use tokio::net::TcpStream;
use tokio_util::codec::{FramedRead, LinesCodec, LinesCodecError};
use crate::web_socket::ClientWebSocket;
use std::collections::HashMap;

#[derive(serde::Serialize)]
//#[derive(Message)]
pub struct Inputs {
    pub number: usize,
    pub states: Vec<u8>,
    pub connected: bool,
}

//#[derive(Message)]
//#[rtype(result = "Response")]
pub struct GetInputs;

impl Message for GetInputs {
    type Result = Inputs;
}

// #[derive(Message)]
// #[rtype(usize)]
// pub struct Subscribe {
//     pub addr: Recipient<Inputs>,
// }
//
// #[derive(Message)]
// #[rtype(result = "()")]
// pub struct Unsubscribe {
//     pub id: usize,
// }

type Framed = FramedWrite<
    String,
    WriteHalf<TcpStream>,
    LinesCodec,
>;

#[derive(Default)]
pub struct RelayActor {
    pub host: String,
    pub port: u16,
    pub inputs_number: usize,
    states: Vec<u8>,
    current_input: usize,
    connected: bool,
    framed: Option<Framed>,
    //sessions: HashMap<usize, Recipient<Inputs>>,
    //pub inputs: web::Data<Mutex<Inputs>>,
    //recipient: Recipient<RelayStatus>,
}

impl Actor for RelayActor {
    type Context = Context<Self>;

    fn started(&mut self, ctx: &mut Self::Context) {
        println!("RelayActor started!");

        Resolver::from_registry()
            .send(Connect::host_and_port(self.host.as_str(), self.port))
            .into_actor(self)
            .map(|res, act, ctx| match res {
                Ok(res2) => match res2 {
                    Ok(stream) => {
                        println!("RelayActor connected!");

                        let (r, w) = split(stream);

                        ctx.add_stream(FramedRead::new(r, LinesCodec::new()));

                        let mut line_writer = actix::io::FramedWrite::new(w, LinesCodec::new(), ctx);

                        line_writer.set_buffer_capacity(0, 10);
                        act.framed = Some(line_writer);
                        act.connected = true;
                    }
                    Err(err) => {
                        println!("RelayActor failed to resolve: {}", err);
                        act.connected = false;
                        ctx.stop();
                    }
                },
                Err(err) => {
                    println!("RelayActor failed to connect: {}", err);
                    act.connected = false;
                    ctx.stop();
                }
            })
            .wait(ctx);

        self.start_poll(ctx);
    }
}

impl StreamHandler<Result<String, LinesCodecError>> for RelayActor {
    fn handle(&mut self, line: Result<String, LinesCodecError>, ctx: &mut Self::Context) {
        match line {
            Ok(line) => {
                if line.starts_with("+OCCH") {
                    let input_no = line
                        .chars()
                        .nth(5)
                        .unwrap()
                        .to_digit(10)
                        .unwrap() as usize;

                    let input_state = line
                        .chars()
                        .nth(7)
                        .unwrap()
                        .to_digit(10)
                        .unwrap() as u8;

                    *&mut self.states[input_no - 1] = input_state;
                }

                println!("{}", line);
            }
            Err(err) => {
                println!("RelayActor error: {}", err);
                ctx.stop();
            }
        }
    }
}

impl RelayActor {
    pub fn new(host: String, port: u16, inputs_number: usize) -> RelayActor {
        RelayActor {
            host,
            port,
            inputs_number,
            current_input: 0,
            states: vec![0u8; inputs_number],
            framed: None,
            connected: false,
            //sessions: HashMap::new(),
        }
    }

    fn start_poll(&self, ctx: &mut <Self as Actor>::Context) {
        ctx.run_interval(Duration::from_millis(200), |act, ctx| {
            if let Some(framed) = act.framed.as_mut() {
                if act.current_input < act.inputs_number {
                    act.current_input += 1;
                    framed.write(format!("AT+OCCH{}=?", act.current_input));
                } else {
                    act.current_input = 0;
                }
            };
        });
    }
}

impl WriteHandler<LinesCodecError> for RelayActor {}

impl Supervised for RelayActor {
    fn restarting(&mut self, ctx: &mut Context<RelayActor>) {
        println!("RelayActor restarting");
    }
}

impl SystemService for RelayActor {
    fn service_started(&mut self, ctx: &mut Context<Self>) {
        println!("WebSocket started");
    }
}

impl Handler<GetInputs> for RelayActor {
    type Result = MessageResult<GetInputs>;

    fn handle(&mut self, _: GetInputs, _: &mut Context<Self>) -> Self::Result {
        MessageResult(Inputs {
            number: self.inputs_number,
            states: self.states.clone(),
            connected: self.connected,
        })
    }
}

// impl Handler<Subscribe> for RelayActor {
//     type Result = usize;
//
//     fn handle(&mut self, msg: Subscribe, _: &mut Context<Self>) -> Self::Result {
//         let id = self.rng.gen::<usize>();
//         self.sessions.insert(id, msg.addr);
//
//         id
//     }
// }
//
// impl Handler<Unsubscribe> for RelayActor {
//     type Result = ();
//
//     fn handle(&mut self, msg: Unsubscribe, _: &mut Context<Self>) {
//         if self.sessions.remove(&msg.id).is_some() {
//         }
//     }
// }