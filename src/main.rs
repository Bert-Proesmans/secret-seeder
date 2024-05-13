use serde::Deserialize;
use socket2::{Domain, SockAddr, Socket, Type};
use std::path::{Path, PathBuf};

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
    let deserialized_struct: Manifest = read_and_deserialize_from_file(&manifest_path)?;
    println!("Deserialized struct: {:?}", deserialized_struct);

    // let listener_address = SockAddr::vsock(libc::VMADDR_CID_HOST, settings.socket_port);
    let listener_address = SockAddr::vsock(libc::VMADDR_CID_LOCAL, settings.socket_port);

    let listener = Socket::new(Domain::VSOCK, Type::STREAM, None)?;
    listener.set_cloexec(true)?;
    listener.bind(&listener_address)?;
    listener.listen(1)?;

    let client_peer_addr = listener.accept()?;
    println!("Client connected: {:?}", client_peer_addr);

    Ok(())
}

fn read_and_deserialize_from_file(path: &PathBuf) -> Result<Manifest, Box<dyn std::error::Error>> {
    // NO MAPPED FILE AND STREAMED DESERIALIZING IN MYYY RUST 2024 ???!!
    let toml_content = std::fs::read_to_string(path)?;
    let manifest = toml::from_str(&toml_content)?;

    Ok(manifest)
}