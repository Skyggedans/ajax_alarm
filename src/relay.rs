use std::any::Any;
use std::borrow::BorrowMut;
use std::collections::{HashMap, VecDeque};
use std::sync::Mutex;
use std::time::Instant;

use actix::actors::resolver::{Connect, Resolver};
use actix::io::{FramedWrite, WriteHandler};
use actix::prelude::*;
use actix::registry::SystemService;
use actix_web::web;
use rand::prelude::*;
use serde::{Deserialize, Serialize};
use tokio::io::{split, WriteHalf};
use tokio::net::TcpStream;
use tokio::sync::oneshot;
use tokio::sync::oneshot::{channel, Receiver, Sender};
use tokio::time::{self, Duration, timeout};
use tokio_util::codec::{FramedRead, LinesCodec, LinesCodecError};
use tokio::macros::support::Pin;
use actix::dev::MessageResponse;

const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(2);
const HEARTBEAT_TIMEOUT: Duration = Duration::from_secs(10);
const OCCH_ALL_PATTERN: &str = "+OCCH_ALL";
const STACH_PATTERN: &str = "+STACH";
const TIME_PATTERN: &str = "+TIME";
const TIMESW_PATTERN: &str = "+TIMESW";


#[derive(Deserialize, Serialize)]
pub struct SystemTime {
    pub date_time: String,
    pub day_of_week: u8,
}

#[derive(Deserialize, Serialize)]
pub struct DailyEvent {
    pub time: String,
    pub state: u32,
}

#[derive(Deserialize, Serialize)]
pub struct CustomEvent {
    pub date_time: String,
    pub state: u32,
}

#[derive(Message)]
#[rtype(result = "Result<SystemTime, ()>")]
pub struct GetSystemTime;

#[derive(Message)]
#[rtype(result = "()")]
pub struct SetSystemTime {
    pub time: SystemTime,
}

#[derive(Message)]
#[rtype(result = "Vec<u32>")]
pub struct GetInputs;

#[derive(Message)]
#[rtype(result = "Result<u32, ()>")]
pub struct GetOutput {
    pub number: usize,
}

#[derive(Message)]
#[rtype(result = "()")]
pub struct SetOutput {
    pub number: usize,
    pub state: u32,
}

#[derive(Message)]
#[rtype(result = "Result<Vec<DailyEvent>, ()>")]
pub struct GetOutputDailySchedule {
    pub number: usize,
}

#[derive(Message)]
#[rtype(result = "()")]
pub struct SetOutputDailySchedule {
    pub number: usize,
    pub event: DailyEvent,
}

#[derive(Message)]
#[rtype(result = "()")]
pub struct ClearOutputDailySchedule {
    pub number: usize,
}

#[derive(Message)]
#[rtype(result = "Result<Vec<CustomEvent>, ()>")]
pub struct GetOutputCustomSchedule {
    pub number: usize,
}

#[derive(Message)]
#[rtype(result = "()")]
pub struct SetOutputCustomSchedule {
    pub number: usize,
    pub event: CustomEvent,
}

#[derive(Message)]
#[rtype(result = "()")]
pub struct ClearOutputCustomSchedule {
    pub number: usize,
}

#[derive(serde::Serialize)]
#[derive(Clone, Message)]
#[rtype(result = "()")]
pub struct RelayStatus {
    pub inputs: Option<Vec<u32>>,
    pub connected: bool,
}

#[derive(Message)]
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
    pub outputs_number: usize,
    inputs: Vec<u32>,
    inputs_mask: u32,
    connected: bool,
    framed: Option<Framed>,
    hb: Instant,
    rng: ThreadRng,
    clients: HashMap<usize, Recipient<RelayStatus>>,
    oneshots: HashMap<String, VecDeque<Sender<Box<dyn Any>>>>,
}

impl Default for RelayActor {
    fn default() -> Self {
        RelayActor {
            host: "".to_string(),
            port: 12345,
            inputs_number: 4,
            outputs_number: 4,
            inputs: vec![0u32; 4],
            inputs_mask: 0xff,
            framed: None,
            connected: false,
            hb: Instant::now(),
            rng: thread_rng(),
            clients: HashMap::new(),
            oneshots: HashMap::new(),
        }
    }
}

impl Actor for RelayActor {
    type Context = Context<Self>;

    fn started(&mut self, ctx: &mut Self::Context) {
        println!("RelayActor started!");

        self.oneshots = HashMap::new();

        Resolver::from_registry()
            .send(Connect::host_and_port(self.host.as_str(), self.port))
            .into_actor(self)
            .map(|res, act, ctx| match res {
                Ok(res) => match res {
                    Ok(stream) => {
                        println!("RelayActor connected!");

                        let (r, w) = split(stream);
                        let mut line_writer = actix::io::FramedWrite::new(w, LinesCodec::new(), ctx);

                        ctx.add_stream(FramedRead::new(r, LinesCodec::new()));
                        line_writer.write("AT+OCMOD=1,100".to_string());
                        act.framed = Some(line_writer);
                        act.connected = true;
                        act.send_status(RelayStatus { inputs: Some(act.inputs.clone()), connected: true });
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
                println!("{}", line);

                if line.starts_with(TIMESW_PATTERN) {
                    self.handle_timesw(line);
                } else if line.starts_with(TIME_PATTERN) {
                    self.handle_time(line);
                } else if line.starts_with(OCCH_ALL_PATTERN) {
                    self.handle_occh_all(line);
                } else if line.starts_with(STACH_PATTERN) {
                    self.handle_stach(line);
                }

                self.hb = Instant::now();
            }
            Err(err) => {
                println!("RelayActor error: {}", err);
                ctx.stop();
            }
        }
    }
}

impl RelayActor {
    pub fn new(host: &str, port: u16, inputs_number: usize, outputs_number: usize) -> Self {
        Self {
            host: String::from(host),
            port,
            inputs_number,
            outputs_number,
            inputs: vec![0u32; inputs_number],
            ..RelayActor::default()
        }
    }

    fn hb(&self, ctx: &mut <Self as Actor>::Context) {
        ctx.run_interval(HEARTBEAT_INTERVAL, |act, ctx| {
            if Instant::now().duration_since(act.hb) > HEARTBEAT_TIMEOUT {
                act.connected = false;

                ctx.run_later(Duration::from_secs(5), |act, ctx| {
                    println!("Relay heartbeat failed, disconnecting!");
                    act.send_status(RelayStatus { inputs: None, connected: false });
                    ctx.stop();
                });
            }
        });
    }

    fn send_status(&self, message: RelayStatus) {
        for (id, addr) in &self.clients {
            addr.do_send(message.clone()).unwrap();
        }
    }

    fn enqueue_oneshot<T: 'static>(&mut self, key: String) -> Pin<Box<impl Future<Output=Result<T, ()>>>> {
        let (sender, receiver) = channel();

        if let Some(vec) = self.oneshots.get_mut(key.as_str()) {
            vec.push_back(sender);
        } else {
            let mut deque = VecDeque::new();

            deque.push_back(sender);
            self.oneshots.insert(key, deque);
        }

        Box::pin(async move {
            let res = time::timeout(Duration::from_secs(10), receiver).await;

            if let Ok(Ok(state)) = res {
                Ok(*state.downcast::<T>().unwrap())
            } else {
                Err(())
            }
        })
    }

    fn fire_oneshot<T: 'static>(&mut self, key: String, value: T) {
        if let Some(vec) = self.oneshots.get_mut(key.as_str()) {
            if let Some(sender) = vec.pop_front() {
                sender.send(Box::new(value)).unwrap_or_else(|err| {
                    println!("Unable to fire oneshot");
                });
            }
        }
    }

    fn handle_time(&mut self, line: String) {
        let value = line.split_at(6).1; // Split on semicolon
        let (date_time, day_of_week) = value.split_at(20); // Split on last space
        let key = format!("{}", TIME_PATTERN);

        let time = SystemTime {
            date_time: date_time.trim().to_string(),
            day_of_week: day_of_week.parse::<u8>().unwrap(),
        };

        self.fire_oneshot(key, time);
    }

    fn handle_timesw(&mut self, line: String) {
        let output_no = line
            .chars()
            .nth(8)
            .unwrap()
            .to_digit(10)
            .unwrap() as usize;

        let mode = line
            .chars()
            .nth(10)
            .unwrap()
            .to_digit(10)
            .unwrap() as u32;

        let split = line.rsplit(',');
        let key = format!("{}:{},{}", TIMESW_PATTERN, output_no, mode);

        let res = match mode {
            1 => {
                let mut events = split.filter(|item| item.len() == 10).map(|item| {
                    let (time, state) = item.split_at(9);

                    DailyEvent {
                        time: time.trim().to_string(),
                        state: state.parse::<u32>().unwrap(),
                    }
                }).collect::<Vec<DailyEvent>>();

                events.reverse();
                self.fire_oneshot(key, events);
            }
            3 => {
                let mut events = split.filter(|item| item.len() == 21).map(|item| {
                    let (date_time, state) = item.split_at(20);

                    CustomEvent {
                        date_time: date_time.trim().to_string(),
                        state: state.parse::<u32>().unwrap(),
                    }
                }).collect::<Vec<CustomEvent>>();

                events.reverse();
                self.fire_oneshot(key, events);
            }
            _ => ()
        };
    }

    fn handle_occh_all(&mut self, line: String) {
        let states_part = line.split_at(10).1;

        let states: Vec<u32> = states_part.split(',')
            .map(|c| c.chars().nth(0).unwrap().to_digit(10).unwrap()).collect();

        let mut states_for_mask = states.clone();
        let mut states_mask = 0;

        states_for_mask.reverse();

        for input in states_for_mask.iter() {
            states_mask <<= 1;
            states_mask += input;
        }

        if states_mask != self.inputs_mask {
            self.inputs_mask = states_mask;
            self.inputs = states.clone();
            self.send_status(RelayStatus { inputs: Some(states), connected: true });
        }
    }

    fn handle_stach(&mut self, line: String) {
        let output_no = line
            .chars()
            .nth(6)
            .unwrap()
            .to_digit(10)
            .unwrap() as usize;

        let output_state = line
            .chars()
            .nth(8)
            .unwrap()
            .to_digit(10)
            .unwrap() as u32;

        let key = format!("{}{}", STACH_PATTERN, output_no);

        self.fire_oneshot(key, output_state);
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

impl Handler<GetSystemTime> for RelayActor {
    type Result = ResponseFuture<Result<SystemTime, ()>>;

    fn handle(&mut self, message: GetSystemTime, _: &mut Context<Self>) -> Self::Result {
        if let Some(ref mut line_writer) = self.framed {
            line_writer.write(format!("AT{}=?", TIME_PATTERN));
        }

        let key = format!("{}", TIME_PATTERN);

        self.enqueue_oneshot(key)
    }
}

impl Handler<SetSystemTime> for RelayActor {
    type Result = ();

    fn handle(&mut self, message: SetSystemTime, _: &mut Context<Self>) -> Self::Result {
        if let Some(ref mut line_writer) = self.framed {
            line_writer.write(format!("AT{}={} {}", TIME_PATTERN, message.time.date_time, message.time.day_of_week));
        }
    }
}

impl Handler<GetInputs> for RelayActor {
    type Result = MessageResult<GetInputs>;

    fn handle(&mut self, _: GetInputs, _: &mut Context<Self>) -> Self::Result {
        MessageResult(self.inputs.clone())
    }
}

impl Handler<GetOutput> for RelayActor {
    type Result = ResponseFuture<Result<u32, ()>>;

    fn handle(&mut self, message: GetOutput, _: &mut Context<Self>) -> Self::Result {
        if let Some(ref mut line_writer) = self.framed {
            line_writer.write(format!("AT{}{}=?", STACH_PATTERN, message.number));
        }

        let key = format!("{}{}", STACH_PATTERN, message.number);

        self.enqueue_oneshot(key)
    }
}

impl Handler<GetOutputDailySchedule> for RelayActor {
    type Result = ResponseFuture<Result<Vec<DailyEvent>, ()>>;

    fn handle(&mut self, message: GetOutputDailySchedule, _: &mut Context<Self>) -> Self::Result {
        if let Some(ref mut line_writer) = self.framed {
            line_writer.write(format!("AT{}={},1?", TIMESW_PATTERN, message.number));
        }

        let key = format!("{}:{},1", TIMESW_PATTERN, message.number);

        self.enqueue_oneshot(key)
    }
}

impl Handler<SetOutputDailySchedule> for RelayActor {
    type Result = ();

    fn handle(&mut self, message: SetOutputDailySchedule, _: &mut Context<Self>) -> Self::Result {
        if let Some(ref mut line_writer) = self.framed {
            line_writer.write(format!("AT{}={},1,{} {}", TIMESW_PATTERN, message.number,
                                      message.event.time, message.event.state));
        }
    }
}

impl Handler<ClearOutputDailySchedule> for RelayActor {
    type Result = ();

    fn handle(&mut self, message: ClearOutputDailySchedule, _: &mut Context<Self>) -> Self::Result {
        if let Some(ref mut line_writer) = self.framed {
            line_writer.write(format!("AT{}={},0", TIMESW_PATTERN, message.number));
        }
    }
}

impl Handler<GetOutputCustomSchedule> for RelayActor {
    type Result = ResponseFuture<Result<Vec<CustomEvent>, ()>>;

    fn handle(&mut self, message: GetOutputCustomSchedule, _: &mut Context<Self>) -> Self::Result {
        if let Some(ref mut line_writer) = self.framed {
            line_writer.write(format!("AT{}={},3?", TIMESW_PATTERN, message.number));
        }

        let key = format!("{}:{},3", TIMESW_PATTERN, message.number);

        self.enqueue_oneshot(key)
    }
}

impl Handler<SetOutputCustomSchedule> for RelayActor {
    type Result = ();

    fn handle(&mut self, message: SetOutputCustomSchedule, _: &mut Context<Self>) -> Self::Result {
        if let Some(ref mut line_writer) = self.framed {
            line_writer.write(format!("AT{}={},3,{} {}", TIMESW_PATTERN, message.number,
                                      message.event.date_time, message.event.state));
        }
    }
}

impl Handler<ClearOutputCustomSchedule> for RelayActor {
    type Result = ();

    fn handle(&mut self, message: ClearOutputCustomSchedule, _: &mut Context<Self>) -> Self::Result {
        if let Some(ref mut line_writer) = self.framed {
            line_writer.write(format!("AT{}={},2", TIMESW_PATTERN, message.number));
        }
    }
}

impl Handler<SetOutput> for RelayActor {
    type Result = ();

    fn handle(&mut self, message: SetOutput, _: &mut Context<Self>) -> Self::Result {
        if let Some(ref mut line_writer) = self.framed {
            line_writer.write(format!("AT{}{}={}", STACH_PATTERN, message.number, message.state));
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

        client.do_send(RelayStatus { inputs: Some(self.inputs.clone()), connected: true });
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
