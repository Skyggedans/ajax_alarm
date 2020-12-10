#![allow(unused_variables)]
#![allow(unused_imports)]

use std::env;
use std::io::{self, Write};
use std::process;
use std::sync::Mutex;

use actix::prelude::*;
use actix::SystemRegistry;
use actix_files as fs;
use actix_web::{
    App, Error, HttpRequest, HttpResponse, HttpServer, Responder, web,
};
use actix_web_actors::ws;
use clap;
use serde_json::json;

#[cfg(target_os = "linux")]
use crate::display::DisplayActor;
#[cfg(target_os = "linux")]
use crate::gpio::GpioActor;
use crate::relay::{RegisterForStatus, RelayActor, GetInputs, SetOutput};
use crate::web_socket::ClientWebSocket;
use actix_web::http::StatusCode;

mod relay;
mod web_socket;

#[cfg(target_os = "linux")]
mod gpio;
#[cfg(target_os = "linux")]
mod display;

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

async fn ws_index(
    r: HttpRequest,
    stream: web::Payload,
) -> Result<HttpResponse, Error> {
    ws::start(ClientWebSocket::new(), &r, stream)
}

async fn inputs() -> HttpResponse {
    let res = RelayActor::from_registry().send(GetInputs).await;

    match res {
        Ok(inputs) => {
            HttpResponse::Ok()
                .content_type("application/json")
                .body(json!({
                "number": inputs.number,
                "states": inputs.states
            }))
        }
        Err(_) => HttpResponse::NoContent().finish()
    }
}

async fn output(web::Path((number, state)): web::Path<(usize, u32)>) -> HttpResponse {
    let res = RelayActor::from_registry().do_send(SetOutput { number, state });

    HttpResponse::Ok().finish()
}

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    let matches = clap::App::new("ajax_alarm")
        .version("1.0")
        .author("Andrei Klaptsov <skyggedanser@gmail.com>")
        .about("WebSocket and GPIO gateway for TCP-KP-I404 and similar network relays")
        .arg(clap::Arg::new("host")
            .short('h')
            .long("relay-host")
            .value_name("HOST")
            .about("Relay host"))
        .arg(clap::Arg::new("port")
            .short('p')
            .long("relay-port")
            .value_name("PORT")
            .about("Relay port, defaults to 12345"))
        .arg(clap::Arg::new("inputs_number")
            .short('i')
            .long("inputs-number")
            .value_name("NUMBER")
            .about("Number of inputs, defaults to 4"))
        .get_matches();

    let mut args = env::args();
    let program = Program::new(args.next().unwrap_or("ajax_alarm".to_string()));

    let host = matches.value_of("host")
        .unwrap_or_else(|| {
            program.print_error("invalid host".to_string());
            program.usage();
            program.fail();
        })
        .to_string();

    let port = matches.value_of("port")
        .unwrap_or("12345")
        .parse::<Port>()
        .unwrap_or_else(|error| {
            program.print_error(format!("invalid port number: {}", error));
            program.usage();
            program.fail();
        });

    let inputs_number = matches.value_of("inputs_number")
        .unwrap_or("4")
        .parse::<usize>()
        .unwrap_or_else(|error| {
            program.print_error(format!("invalid inputs number: {}", error));
            program.usage();
            program.fail();
        });

    let relay = Supervisor::start(move |_| RelayActor::new(host, port, inputs_number));

    SystemRegistry::set(relay.clone());

    #[cfg(target_os = "linux")] {
        let gpio = GpioActor::new().start();
        let display = DisplayActor::new().start();
    }

    HttpServer::new(move || {
        App::new()
            .service(web::resource("/ws/").route(web::get().to(ws_index)))
            .route("/inputs", web::get().to(inputs))
            .route("/output/{number}/{state}", web::post().to(output))
            .service(fs::Files::new("/", "static/").index_file("index.html"))
    })
        .bind("0.0.0.0:8080")?
        .run()
        .await
}
