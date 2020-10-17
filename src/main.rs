#![allow(unused_variables)]
#![allow(unused_imports)]

use actix::prelude::*;
use actix_files as fs;
use actix_web::{
    get, middleware, post, web, App, Error, HttpRequest, HttpResponse, HttpServer, Responder,
};
use actix_web_actors::ws;
use serde_json::json;
use std::env;
use std::io::{self, Read, Write};
use std::net::TcpListener;
use std::net::TcpStream;
use std::process;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};
//use sysfs_gpio::{Direction, Pin};

const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(5);
const CLIENT_TIMEOUT: Duration = Duration::from_secs(10);
const OPTOCOUPLE_PIN_GPIO_NO: u8 = 7;

type Port = u16;

struct Program {
    name: String,
}

impl Program {
    fn new(name: String) -> Program {
        Program { name }
    }

    fn usage(&self) {
        println!("usage: {} HOST PORT", self.name);
    }

    fn print_error(&self, msg: String) {
        writeln!(io::stderr(), "{}: error: {}", self.name, msg);
    }

    fn print_fail(&self, msg: String) -> ! {
        self.print_error(msg);
        self.fail();
    }

    fn exit(&self, status: i32) -> ! {
        process::exit(status);
    }

    fn fail(&self) -> ! {
        self.exit(-1);
    }
}

struct Inputs {
    number: usize,
    states: Vec<u8>,
}

struct AppState {
    inputs: Mutex<Inputs>,
}

struct ClientWebSocket {
    hb: Instant,
    data: web::Data<AppState>,
}

impl Actor for ClientWebSocket {
    type Context = ws::WebsocketContext<Self>;

    fn started(&mut self, ctx: &mut Self::Context) {
        self.hb(ctx);
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
    fn new(data: web::Data<AppState>) -> Self {
        Self {
            hb: Instant::now(),
            data,
        }
    }

    fn hb(&self, ctx: &mut <Self as Actor>::Context) {
        ctx.run_interval(HEARTBEAT_INTERVAL, |act, ctx| {
            // check client heartbeats
            if Instant::now().duration_since(act.hb) > CLIENT_TIMEOUT {
                // heartbeat timed out
                println!("Websocket Client heartbeat failed, disconnecting!");
                ctx.stop();

                return;
            }

            ctx.ping(b"");

            let inputs = act.data.inputs.lock().unwrap();
            ctx.text(json!(inputs.states).to_string());
        });
    }
}

async fn ws_index(
    r: HttpRequest,
    stream: web::Payload,
    data: web::Data<AppState>,
) -> Result<HttpResponse, Error> {
    ws::start(ClientWebSocket::new(data), &r, stream)
}

async fn inputs(data: web::Data<AppState>) -> impl Responder {
    let inputs = data.inputs.lock().unwrap();

    HttpResponse::Ok().body(json!({
        "number": inputs.number,
        "states": inputs.states
    }))
}

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    let mut args = env::args();
    let program = Program::new(args.next().unwrap_or("ajax_alarm".to_string()));

    let host = args.next().unwrap_or_else(|| {
        program.usage();
        program.fail();
    });

    let port = args
        .next()
        .unwrap_or_else(|| {
            program.usage();
            program.fail();
        })
        .parse::<Port>()
        .unwrap_or_else(|error| {
            program.print_error(format!("invalid port number: {}", error));
            program.usage();
            program.fail();
        });

    let input_number = args
        .next()
        .unwrap_or("4".to_string())
        .parse::<usize>()
        .unwrap_or_else(|error| {
            program.print_error(format!("invalid input number: {}", error));
            program.usage();
            program.fail();
        });

    let mut stream = TcpStream::connect((host.as_str(), port))
        .unwrap_or_else(|error| program.print_fail(error.to_string()));

    let mut input_stream = stream.try_clone().unwrap();

    let app_state = web::Data::new(AppState {
        inputs: Mutex::new(Inputs {
            number: input_number,
            states: Vec::new(),
        }),
    });

    let app_state_clone = app_state.clone();

    // TX thread
    thread::spawn(move || {
        let mut client_buffer = [0u8; 1024];
        //let my_led = Pin::new(OPTOCOUPLE_PIN_GPIO_NO);
        let mut input_states = vec![0u8; input_number];

        loop {
            //my_led.set_value(0).unwrap();
            thread::sleep(Duration::from_millis(100));
            match input_stream.read(&mut client_buffer) {
                Ok(n) => {
                    if n == 0 {
                        program.exit(0);
                    } else {
                        let received_string =
                            std::str::from_utf8(&client_buffer).unwrap().to_string();

                        if received_string.starts_with("+OCCH") {
                            let input_no = received_string
                                .chars()
                                .nth(5)
                                .unwrap()
                                .to_digit(10)
                                .unwrap() as usize;
                            let input_state = received_string
                                .chars()
                                .nth(7)
                                .unwrap()
                                .to_digit(10)
                                .unwrap() as u8;

                            *&mut input_states[input_no - 1] = input_state;
                        }

                        let mut inputs = app_state_clone.inputs.lock().unwrap();

                        inputs.states = input_states.to_vec();
                        io::stdout().write(&received_string.as_bytes()).unwrap();
                        io::stdout().flush().unwrap();
                        //my_led.set_value(1).unwrap();

                        thread::sleep(Duration::from_millis(100));
                    }
                }
                Err(error) => program.print_fail(error.to_string()),
            }
        }
    });

    // RX thread
    thread::spawn(move || {
        let output_stream = &mut stream;

        loop {
            for input in 1..=input_number {
                output_stream
                    .write(format!("AT+OCCH{}=?\n", input).as_bytes())
                    .unwrap();

                output_stream.flush().unwrap();
                thread::sleep(Duration::from_millis(100));
            }

            thread::sleep(Duration::from_millis(1000));
        }
    });

    // std::env::set_var("RUST_LOG", "actix_server=info,actix_web=info");
    // env_logger::init();
    HttpServer::new(move || {
        App::new()
            // enable logger
            //.wrap(middleware::Logger::default())
            // websocket route
            .app_data(app_state.clone())
            .service(web::resource("/ws/").route(web::get().to(ws_index)))
            .route("/inputs", web::get().to(inputs))
            .service(fs::Files::new("/", "static/").index_file("index.html"))
    })
    .bind("0.0.0.0:8080")?
    .run()
    .await
}
