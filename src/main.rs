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
use actix_web::http::StatusCode;
use actix_web_actors::ws;
use clap;
use serde_json::json;

#[cfg(target_os = "linux")]
use crate::display::DisplayActor;
#[cfg(target_os = "linux")]
use crate::gpio::GpioActor;
use crate::relay::{GetInputs, GetOutput, GetOutputSchedule, GetSystemTime, RegisterForStatus, RelayActor, SetOutput, SetSystemTime, SystemTime};
use crate::web_socket::ClientWebSocket;

mod relay;
mod web_socket;

#[cfg(target_os = "linux")]
mod gpio;
#[cfg(target_os = "linux")]
mod display;

type Port = u16;

struct Program {
    pub host: String,
    pub port: u16,
    pub inputs_number: usize,
    pub outputs_number: usize,
}

impl Program {
    fn new() -> Program {
        let mut clap = clap::App::new("ajax_alarm");

        let matches = clap.clone()
            .version("1.0")
            .author("Skyggedans <skyggedanser@gmail.com>")
            .about("WebSocket, GPIO gateway and display terminal for TCP-KP-I404 and similar network relays by Guangzhou Niren (Clayman) Electronic Technology Co., Ltd.")
            .arg(clap::Arg::new("host")
                .short('r')
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
            .arg(clap::Arg::new("outputs_number")
                .short('o')
                .long("outputs-number")
                .value_name("NUMBER")
                .about("Number of outputs, defaults to 4"))
            .get_matches();

        let host = matches.value_of("host")
            .unwrap_or_else(|| {
                Program::print_error("invalid host".to_string());
                clap.write_long_help(&mut io::stdout()).unwrap();
                process::exit(-1);
            })
            .to_string();

        let port = matches.value_of("port")
            .unwrap_or("12345")
            .parse::<Port>()
            .unwrap_or_else(|error| {
                Program::print_error(format!("invalid port number: {}", error));
                clap.write_long_help(&mut io::stdout()).unwrap();
                process::exit(-1);
            });

        let inputs_number = matches.value_of("inputs_number")
            .unwrap_or("4")
            .parse::<usize>()
            .unwrap_or_else(|error| {
                Program::print_error(format!("invalid inputs number: {}", error));
                clap.write_long_help(&mut io::stdout()).unwrap();
                process::exit(-1);
            });

        let outputs_number = matches.value_of("outputs_number")
            .unwrap_or("4")
            .parse::<usize>()
            .unwrap_or_else(|error| {
                Program::print_error(format!("invalid outputs number: {}", error));
                clap.write_long_help(&mut io::stdout()).unwrap();
                process::exit(-1);
            });

        Program {
            host,
            port,
            inputs_number,
            outputs_number,
        }
    }

    fn print_error(msg: String) {
        writeln!(io::stderr(), "error: {}", msg).unwrap();
    }
}

async fn ws_index(
    r: HttpRequest,
    stream: web::Payload,
) -> Result<HttpResponse, Error> {
    ws::start(ClientWebSocket::new(), &r, stream)
}

async fn get_system_time() -> HttpResponse {
    let res = RelayActor::from_registry().send(GetSystemTime {}).await;

    if let Ok(Ok(res)) = res {
        HttpResponse::Ok()
            .content_type("application/json")
            .body(json!(res))
    } else {
        HttpResponse::NoContent().finish()
    }
}

async fn set_system_time(json: web::Json<SystemTime>) -> HttpResponse {
    let res = RelayActor::from_registry().do_send(SetSystemTime { time: json.0 });

    HttpResponse::Ok().finish()
}

async fn inputs() -> HttpResponse {
    let res = RelayActor::from_registry().send(GetInputs).await;

    if let Ok(res) = res {
        HttpResponse::Ok()
            .content_type("application/json")
            .body(json!(res))
    } else {
        HttpResponse::NoContent().finish()
    }
}

async fn get_output(web::Path(number): web::Path<usize>) -> HttpResponse {
    let res = RelayActor::from_registry().send(GetOutput { number }).await;

    if let Ok(Ok(res)) = res {
        HttpResponse::Ok()
            .content_type("application/json")
            .body(json!(res))
    } else {
        HttpResponse::NoContent().finish()
    }
}

async fn set_output(web::Path((number, state)): web::Path<(usize, u32)>) -> HttpResponse {
    let res = RelayActor::from_registry().do_send(SetOutput { number, state });

    HttpResponse::Ok().finish()
}

async fn get_output_schedule(web::Path((number, mode)): web::Path<(usize, u8)>) -> HttpResponse {
    let res = RelayActor::from_registry().send(GetOutputSchedule { number, mode }).await;

    if let Ok(Ok(res)) = res {
        HttpResponse::Ok()
            .content_type("application/json")
            .body(json!(res))
    } else {
        HttpResponse::NoContent().finish()
    }
}

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    let program = Program::new();
    let relay = Supervisor::start(move |_| RelayActor::new(program.host, program.port,
                                                           program.inputs_number, program.outputs_number));

    SystemRegistry::set(relay.clone());

    #[cfg(target_os = "linux")] {
        let gpio = GpioActor::new().start();
        let display = DisplayActor::new().start();
    }

    HttpServer::new(move || {
        App::new()
            .service(web::resource("/ws/").route(web::get().to(ws_index)))
            .route("/system_time", web::get().to(get_system_time))
            .route("/system_time", web::post().to(set_system_time))
            .route("/inputs", web::get().to(inputs))
            .route("/output/{number}", web::get().to(get_output))
            .route("/output/{number}/{state}", web::post().to(set_output))
            .route("/output/{number}/schedule/{mode}", web::get().to(get_output_schedule))
            .service(fs::Files::new("/", "static/").index_file("index.html"))
    })
        .bind("0.0.0.0:8080")?
        .run()
        .await
}
