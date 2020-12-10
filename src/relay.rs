use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use actix::actors::resolver::{Connect, Resolver};
use actix::io::{FramedWrite, WriteHandler};
use actix::prelude::*;
use actix::registry::SystemService;
use actix::WeakAddr;
use actix_web::web;
use rand::prelude::*;
use tokio::io::{split, WriteHalf};
use tokio::net::TcpStream;
use tokio_util::codec::{FramedRead, LinesCodec, LinesCodecError};

const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(2);
const HEARTBEAT_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(serde::Serialize)]
pub struct Inputs {
    pub number: usize,
    pub states: Vec<u32>,
    pub connected: bool,
}

pub struct GetInputs;

impl Message for GetInputs {
    type Result = Inputs;
}

#[derive(Message)]
#[rtype(result = "()")]
pub struct SetOutput {
    pub number: usize,
    pub state: u32,
}

#[derive(serde::Serialize)]
#[derive(Message)]
#[rtype(result = "()")]
pub struct RelayStatus {
    pub inputs: Vec<u32>,
    pub connected: bool,
}

#[derive(Message, Debug)]
#[rtype(result = "usize")]
pub struct RegisterForStatus(pub Recipient<RelayStatus>);

#[derive(Message)]
#[rtype(result = "()")]
pub struct UnregisterForStatus(pub usize);

type Framed = FramedWrite<
    String,
    WriteHalf<TcpStream>,
    LinesCodec,
>;

pub struct RelayActor {
    pub host: String,
    pub port: u16,
    pub inputs_number: usize,
    states: Vec<u32>,
    connected: bool,
    framed: Option<Framed>,
    hb: Instant,
    rng: ThreadRng,
    clients: HashMap<usize, Recipient<RelayStatus>>,
}

impl Default for RelayActor {
    fn default() -> Self {
        RelayActor {
            host: "".to_string(),
            port: 12345,
            inputs_number: 4,
            states: vec![0u32; 4],
            framed: None,
            connected: false,
            hb: Instant::now(),
            rng: thread_rng(),
            clients: HashMap::new(),
        }
    }
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
                        let mut line_writer = actix::io::FramedWrite::new(w, LinesCodec::new(), ctx);

                        ctx.add_stream(FramedRead::new(r, LinesCodec::new()));
                        //line_writer.set_buffer_capacity(0, 10);
                        line_writer.write("AT+OCMOD=1,100".to_string());
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

        self.hb(ctx);
    }
}

impl StreamHandler<Result<String, LinesCodecError>> for RelayActor {
    fn handle(&mut self, line: Result<String, LinesCodecError>, ctx: &mut Self::Context) {
        match line {
            Ok(line) => {
                if line.starts_with("+OCCH_ALL") {
                    let states_part = line.split_at(10).1;

                    let states: Vec<u32> = states_part.split(',')
                        .map(|c| c.chars().nth(0).unwrap().to_digit(10).unwrap()).collect();

                    self.states = states;
                }

                self.send_status();
                self.hb = Instant::now();
                //println!("{}", line);
            }
            Err(err) => {
                println!("RelayActor error: {}", err);
                ctx.stop();
            }
        }
    }
}

impl RelayActor {
    pub fn new(host: String, port: u16, inputs_number: usize) -> Self {
        Self {
            host,
            port,
            inputs_number,
            ..RelayActor::default()
        }
    }

    fn hb(&self, ctx: &mut <Self as Actor>::Context) {
        ctx.run_interval(HEARTBEAT_INTERVAL, |act, ctx| {
            if Instant::now().duration_since(act.hb) > HEARTBEAT_TIMEOUT {
                act.connected = false;

                ctx.run_later(Duration::from_secs(5), |_, ctx| {
                    println!("Relay heartbeat failed, disconnecting!");
                    ctx.stop();
                });
            }
        });
    }

    fn send_status(&self) {
        for (id, addr) in &self.clients {
            let message = RelayStatus { inputs: self.states.clone(), connected: self.connected };

            addr.do_send(message).unwrap();
        }
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
        println!("RelayActor started");
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

impl Handler<SetOutput> for RelayActor {
    type Result = ();

    fn handle(&mut self, message: SetOutput, _: &mut Context<Self>) -> Self::Result {
        if let Some(ref mut line_writer) = self.framed {
            line_writer.write(format!("AT+STACH{}={}", message.number, message.state));
        }
    }
}

impl Handler<RegisterForStatus> for RelayActor {
    type Result = usize;

    fn handle(
        &mut self,
        RegisterForStatus(client): RegisterForStatus,
        _ctx: &mut Self::Context,
    ) -> Self::Result {
        let id = self.rng.gen::<usize>();

        self.clients.insert(id, client);

        id
    }
}

impl Handler<UnregisterForStatus> for RelayActor {
    type Result = ();

    fn handle(&mut self, UnregisterForStatus(id): UnregisterForStatus, _ctx: &mut Context<Self>) {
        self.clients.remove(&id);
    }
}
