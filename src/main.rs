const HELP: &str = "\
bss [--port <u32>] [--timeout <u32>] [--help] [COMMAND] MANIFEST_FILE_PATH

OPTIONS:
    --vsock-address <TODO>
    --unix-socket   <TODO>
    --ip-address    <TODO>
    -p, --port      The port number to listen/connect to. (Default {})

    -t, --timeout   The amount of seconds to block waiting until a succesful connection is setup between sender and receiver. (Default {})
    -b, --bytes-max The per-transferred-file maximum byte size limit. (Default {})
    --help          Print this help message and exit.

COMMANDS:
    send        Connects to another process started with subcommand 'receive' to send files according to the manifest.

    receive     Opens a new socket to receive and store files according to the manifest.

NOTE: The connection addresses are tried in the order VSOCK network > UNIX socket > IP network. The first argument provided in that order will be used for creating a connection.
ERROR: UNIX sockets will not work on non-UNIX operating systems.
";

// TODO Constants

// A port that requires CAP_NET_ADMIN to bind to, because the datastream
// will contain sensitive material.
//
// The port can be set to another value, please do so when you understand
// the threat model of leaking sensitive secrets.
const DEFAULT_LISTEN_ADDRESS: u32 = 21;

// A default connect timeout because everything needs a lifetime.
// The value is in unit seconds.
const DEFAULT_TIMEOUT: u32 = 3600;

// A default for the maximum filesize to transfer. Security measure to protect
// the receive side.
const DEFAULT_MAX_TRASMISSION_BYTES: u32 = 1024 * 1024;

type StateType = ();

#[derive(serde::Deserialize, Debug)]
struct Manifest {
    secrets: Vec<Secret>,
}

#[derive(serde::Deserialize, Debug)]
struct Secret {
    name: String,
    source_path: std::path::PathBuf,
    destination_path: std::path::PathBuf,
    owner: String,
    group: String,
    mode: String,
}

struct GlobalSettings {
    timeout_seconds: u32,
    socket_port: u32,
    max_transmission_bytes: u32,
}

// Implements the receive side, aka the HTTP (and connection) server.
mod receive;

// Implements the send side, aka the HTTP client.
mod send;

#[cfg(unix)]
// Implements unix sockets as underlying transport mechanism.
mod unix_socket;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    use lexopt::prelude::*;

    pretty_env_logger::init();

    let mut settings = GlobalSettings {
        timeout_seconds: DEFAULT_TIMEOUT,
        socket_port: DEFAULT_LISTEN_ADDRESS,
        max_transmission_bytes: DEFAULT_MAX_TRASMISSION_BYTES,
    };

    let mut parser = lexopt::Parser::from_env();
    while let Some(token) = parser.next()? {
        match token {
            Short('h') | Long("help") => {
                println!("{}", HELP);
                std::process::exit(0);
            }
            Short('t') | Long("timeout") => {
                settings.timeout_seconds = parser.value()?.parse()?;
            }
            Short('p') | Long("port") => {
                settings.socket_port = parser.value()?.parse()?;
            }
            Short('b') | Long("bytes-max") => {
                settings.max_transmission_bytes = parser.value()?.parse()?;
            }
            Value(value) => {
                let value = value.string()?;
                match value.as_str() {
                    "receive" => {
                        return receive::server_main(settings, parser);
                    }
                    "send" => {
                        return send::client_main(settings, parser);
                    }
                    value => {
                        return Err(format!("unknown subcommand '{}'", value).into());
                    }
                }
            }
            _ => return Err(token.unexpected())?,
        }
    }

    println!("{}", HELP);
    Ok(())
}

fn read_and_deserialize_manifest(_path: std::path::PathBuf) -> &'static mut Manifest {
    // TODO Actually parse provided manifest

    let example_manifest = Manifest {
        secrets: vec![Secret {
            name: "test".to_string(),
            source_path: std::path::PathBuf::from(r"/tmp/source"),
            destination_path: std::path::PathBuf::from(r"/tmp/target"),
            owner: "bert-proesmans".to_string(),
            group: "bert-proesmans".to_string(),
            mode: "0664".to_string(),
        }],
    };

    Box::leak(Box::new(example_manifest))
}
