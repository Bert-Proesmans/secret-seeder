//#![deny(warnings)]

use hyper::{body::{Body, Bytes, Frame}, server::conn::http1::Builder, service::service_fn, Method, Request, Response, StatusCode};
use http_body_util::{combinators::BoxBody, BodyExt, Empty, Full};
use serde::Deserialize;
use socket2::{Domain, SockAddr, Socket, Type};
use std::path::{PathBuf};

const HELP: &str = "\
bss [--port <u32>] [--timeout <u32>] [--help] [COMMAND] MANIFEST_FILE_PATH

OPTIONS:
    -p, --port      The port number to listen/connect to.
    -t, --timeout   The amount of seconds to block waiting until a succesful connection is setup between sender and receiver.
    --help          Print this help message and exit.

COMMANDS:
    send        Connects to another process started with subcommand 'receive' to send files according to the manifest.

    receive     Opens a new socket to receive and store files according to the manifest.
";

// A port that requires CAP_NET_ADMIN to bind to, because the datastream
// will contain sensitive material.
//
// The port can be set to another value, please do so when you understand
// the threat model of leaking sensitive secrets.
const DEFAULT_LISTEN_ADDRESS: u32 = 21;

// A default connect timeout because everything needs a lifetime.
// The value is in unit seconds.
const DEFAULT_TIMEOUT: u32 = 3600;

#[derive(Deserialize, Debug)]
struct Manifest {
    secrets: Vec<Secret>,    
}

#[derive(Deserialize, Debug)]
struct Secret {
    name: String,
    source_path: PathBuf,
    destination_path: PathBuf,
    owner: String,
    group: String,
    mode: String
}

struct GlobalSettings {
    timeout_seconds: u32,
    socket_port: u32,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    use lexopt::prelude::*;

    let mut settings = GlobalSettings {
        timeout_seconds: DEFAULT_TIMEOUT,
        socket_port: DEFAULT_LISTEN_ADDRESS,
    };

    let mut parser = lexopt::Parser::from_env();
    while let Some(token) =parser.next()? {
        match token {
            Short('h') | Long("help") => {
                println!("{}", HELP);
                std::process::exit(0);
            },
            Short('t') | Long("timeout") => {
                settings.timeout_seconds = parser.value()?.parse()?;
            },
            Short('p') | Long("port") => {
                settings.socket_port = parser.value()?.parse()?;
            }
            Value(value) => {
                let value = value.string()?;
                match value.as_str() {
                    "receive" => {
                        return receive(settings, parser);
                    },
                    "send" => {
                        return send(settings, parser);
                    },
                    value => {
                        return Err(format!("unknown subcommand '{}'", value).into());
                    }
                }
            }
            _ => return Err(token.unexpected())?
        }
    }

    println!("{}", HELP);
    Ok(())
}

fn read_and_deserialize_from_file(path: &PathBuf) -> Result<Manifest, Box<dyn std::error::Error>> {
    // NO MAPPED FILE AND STREAMED DESERIALIZING IN MYYY RUST 2024 ???!!
    let toml_content = std::fs::read_to_string(path)?;
    let manifest = toml::from_str(&toml_content)?;

    Ok(manifest)
}

fn empty() -> BoxBody<Bytes, hyper::Error> {
    Empty::<Bytes>::new()
        .map_err(|never| match never {})
        .boxed()
}

fn full<T: Into<Bytes>>(chunk: T) -> BoxBody<Bytes, hyper::Error> {
    Full::new(chunk.into())
        .map_err(|never| match never {})
        .boxed()
}

fn send(settings: GlobalSettings, mut parser: lexopt::Parser) -> Result<(), Box<dyn std::error::Error>> {
    use lexopt::prelude::*;
    use std::time::Duration;

    let mut manifest_path = None;
    let mut computer_id = None::<u32>;

    while let Some(arg) = parser.next()? {
        match arg {
            Value(value) if manifest_path.is_none() => {
                manifest_path = Some(value.into());
            },
            Value(value) if computer_id.is_none() => {
                computer_id = Some(value.parse()?);
            }
            _ => return Err(arg.unexpected())?
        }
    }

    let manifest_path = manifest_path.ok_or("Missing path to the manifest file")?;
    let deserialized_struct: Manifest = read_and_deserialize_from_file(&manifest_path)?;
    println!("Deserialized struct: {:?}", deserialized_struct);

    let computer_id = computer_id.ok_or("Missing computer ID to connect to")?;

    // let connect_address = SockAddr::vsock(computer_id, settings.socket_port);
    let connect_address = SockAddr::vsock(libc::VMADDR_CID_LOCAL, settings.socket_port);

    let connect = Socket::new(Domain::VSOCK, Type::STREAM, None)?;
    connect.set_cloexec(true)?;
    connect.connect_timeout(&connect_address, Duration::from_secs(settings.timeout_seconds.into()))?;
    println!("Connected to server: {:?}", connect.peer_addr()?);

    // Configure a runtime for the server that runs everything on the current thread
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("build runtime");

    // Combine it with a `LocalSet,  which means it can spawn !Send futures...
    let local = tokio::task::LocalSet::new();
    //local.block_on(&rt, http2_server()).unwrap();

    Ok(())
}

fn receive(settings: GlobalSettings, mut parser: lexopt::Parser) -> Result<(), Box<dyn std::error::Error>> {
    use lexopt::prelude::*;

    let mut manifest_path = None;
    while let Some(arg) = parser.next()? {
        match arg {
            Value(value) if manifest_path.is_none() => {
                manifest_path = Some(value.into());
            },
            _ => return Err(arg.unexpected())?
        }
    }

    let manifest_path = manifest_path.ok_or("Missing path to the manifest file")?;
    let manifest: Manifest = read_and_deserialize_from_file(&manifest_path)?;
    println!("Deserialized struct: {:?}", manifest);

    // let listener_address = SockAddr::vsock(libc::VMADDR_CID_HOST, settings.socket_port);
    let listener_address = SockAddr::vsock(libc::VMADDR_CID_LOCAL, settings.socket_port);

    let listener = Socket::new(Domain::VSOCK, Type::STREAM, None)?;
    listener.set_cloexec(true)?;
    listener.bind(&listener_address)?;
    listener.listen(1)?;

    let client = listener.accept()?;
    let client_addr = client.1;
    println!("Client connected: {:?}", client_addr);

    // TODO; Verify connection came from hypervisor

    
    // Configure a runtime for the server that runs everything on the current thread
    let rt = tokio::runtime::Builder::new_current_thread()
    .enable_all()
    .build()
    .expect("build runtime");

// Combine it with a `LocalSet,  which means it can spawn !Send futures...
let local = tokio::task::LocalSet::new();
local.block_on(&rt, (|| async move {
        let http_server = Builder::new();
        http_server.serve_connection(todo!(), service_fn(|req| async move {
            receive_posts(req, &manifest)
        })).await;
    })())?;


    Ok(())
}

fn parse_tag(path: &str) -> Option<&str> {
    if path.starts_with("/secrets/") {
        Some(&path[9..])
    } else {
        None
    }
}

fn error_not_found() -> Result<Response<BoxBody<Bytes, hyper::Error>>, hyper::Error> {
    let mut not_found = Response::new(empty());
    *not_found.status_mut() = StatusCode::NOT_FOUND;
    Ok(not_found)
}

async fn receive_posts(req: Request<hyper::body::Incoming>, manifest: &Manifest) -> Result<Response<BoxBody<Bytes, hyper::Error>>, hyper::Error> {
    match (req.method(), req.uri().path()) {
        // Serve some instructions at /
        (&Method::GET, "/") => Ok(Response::new(full(
            "Try POSTing data to /secrets/<tagname> such as: `curl localhost:3000/secrets/mysecret -XPOST -d \"hello world\"`",
        ))),

        (&Method::POST, path) => {
            if let Some(tag) = parse_tag(path) {
                if let Some(secret) = manifest.secrets.iter().find(|&item| item.name == tag) {
                    // To protect our server, reject requests with bodies larger than
                    // 64kbs of data.
                    let max = req.body().size_hint().upper().unwrap_or(u64::MAX);
                    if max > 1024 * 64 {
                        let mut resp = Response::new(full("Body too big"));
                        *resp.status_mut() = hyper::StatusCode::PAYLOAD_TOO_LARGE;
                        return Ok(resp);
                    }

                    println!("Reading file contents for tag {}", tag);
                    let whole_body = req.collect().await?.to_bytes();
                    
                    // TODO STORE

                    let mut ok_no_response = Response::new(empty());
                    *ok_no_response.status_mut() = StatusCode::NO_CONTENT;
                    return Ok(ok_no_response);
                }
            }

            error_not_found()
        },

        // Return the 404 Not Found for other routes.
        _ => error_not_found(),
    }
}
