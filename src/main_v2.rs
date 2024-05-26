use http_body_util::combinators::BoxBody;
use http_body_util::{BodyExt, Empty, Full};
use httparse::Status;
use hyper::server::conn::http2;
use serde::Deserialize;
use std::cell::Cell;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::rc::Rc;
use tokio::io::{self, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::mpsc::{self, Sender};

use hyper::body::{Body, Bytes, Frame};
use hyper::service::service_fn;
use hyper::StatusCode;
use hyper::{Error, Response};
use hyper::{Method, Request, Version};
use std::marker::PhantomData;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use std::thread;
use tokio::net::TcpStream;

mod tokio_runtime;
use tokio_runtime::TokioIo;

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
    mode: String,
}

fn main() {
    let manifest: &'static _ = read_and_deserialize_manifest();

    let server_http1 = thread::spawn(move || {
        // Configure a runtime for the server that runs everything on the current thread
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("build runtime");

        // Combine it with a `LocalSet,  which means it can spawn !Send futures...
        let local = tokio::task::LocalSet::new();
        local
            .block_on(&rt, http1_server(manifest))
            .unwrap();
    });

    let client_http1 = thread::spawn(move || {
        // Configure a runtime for the client that runs everything on the current thread
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("build runtime");

        // Combine it with a `LocalSet,  which means it can spawn !Send futures...
        let local = tokio::task::LocalSet::new();
        local
            .block_on(
                &rt,
                http1_client(
                    "http://localhost:3001".parse::<hyper::Uri>().unwrap(),
                    manifest,
                ),
            )
            .unwrap();
    });

    server_http1.join().unwrap();
    client_http1.join().unwrap();
}

fn read_and_deserialize_manifest() -> &'static mut Manifest {
    let example_manifest = Manifest {
        secrets: vec![Secret {
            name: "test".to_string(),
            source_path: PathBuf::from(r"/tmp/source"),
            destination_path: PathBuf::from(r"/tmp/source"),
            owner: "bert-proesmans".to_string(),
            group: "bert-proesmans".to_string(),
            mode: "0664".to_string(),
        }],
    };

    Box::leak(Box::new(example_manifest))
}

fn empty() -> BoxBody<Bytes, hyper::Error> {
    Empty::<Bytes>::new()
        .map_err(|never| match never {})
        .boxed()
}

fn full_body<T: Into<Bytes>>(chunk: T) -> BoxBody<Bytes, hyper::Error> {
    Full::new(chunk.into())
        .map_err(|never| match never {})
        .boxed()
}

fn error_not_found() -> Result<Response<BoxBody<Bytes, hyper::Error>>, hyper::http::Error> {
    Response::builder()
        .status(StatusCode::NOT_FOUND)
        .body(empty())
}

fn ok_created() -> Result<Response<BoxBody<Bytes, hyper::Error>>, hyper::http::Error> {
    Response::builder()
        .status(StatusCode::CREATED)
        .body(empty())
}

async fn http1_server(manifest: &'static Manifest) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let addr = SocketAddr::from(([127, 0, 0, 1], 3001));

    let listener = TcpListener::bind(addr).await?;
    let state = Arc::new(());
    let (shutdown_signal, mut shutdown_slot) = mpsc::channel::<()>(1);

    'server: loop {
        // NOTE; Move state into loop
        // NOTE; Each iteration needs a copy of the state because the variables are moved out of the loop
        let state = state.clone();
        let shutdown_signal = shutdown_signal.clone();

        tokio::select! {
            // Always test for shutdown first
            biased;

            _ = shutdown_slot.recv() => {
                break 'server Ok(());
            }

            result = listener.accept() => {
                match result {
                    Ok((stream, _)) => {
                        let io = TokioIo::new(stream);

                        let service = service_fn(move |request| {
                            // TODO Client address must be VSOCK HYPERVISOR

                            // ERROR; Keep the clone calls on these state variables.
                            // NOTE; I'm not sure why cloning here is necessary..
                            request_router(state.clone(), manifest, shutdown_signal.clone(), request)
                        });

                        tokio::task::spawn_local(async move {
                        if let Err(err) = hyper::server::conn::http1::Builder::new()
                            .serve_connection(io, service)
                            .await
                        {
                            println!("Error serving connection: {:?}", err);
                        }
                    });
                    },
                    Err(e) => {
                        break 'server Err(e)?;
                    }
                }
            }
        };
    }
}

async fn request_router(
    state: Arc<()>,
    manifest: &Manifest,
    shutdown_signal: Sender<()>,
    request: Request<hyper::body::Incoming>,
) -> Result<Response<BoxBody<Bytes, hyper::Error>>, Box<dyn std::error::Error + Send + Sync>> {
    match (request.method(), request.uri().path()) {
        (&Method::GET, "/") => {
            let message = r#"Listening. POST data to /secrets/<tagname> endpoint.
            example: curl <host>/secrets/mysecret -X POST --header "Content-Type: text/plain" --data-binary "@path/to/file"
            "#;
            Ok(Response::new(full_body(message)))
        }
        (&Method::POST, path) if path.starts_with("/secrets/") => {
            let tag = &path[9..];
            let secret = match manifest.secrets.iter().find(|&item| item.name == tag) {
                Some(secret) => secret,
                None => return error_not_found().map_err(Into::into)
            };

            // TODO Validate body size
            // TODO Shutdown stream if receiving data takes too long
            let file_data = request.collect().await?.to_bytes();
            // TODO Store file

            shutdown_signal.send(()).await?;
            ok_created().map_err(Into::into)
        }
        _ => error_not_found().map_err(Into::into),
    }
}

async fn http1_client(
    url: hyper::Uri,
    manifest: &Manifest,
) -> Result<(), Box<dyn std::error::Error>> {
    let host = url.host().expect("uri has no host");
    let port = url.port_u16().unwrap_or(80);
    let addr = format!("{}:{}", host, port);
    let stream = TcpStream::connect(addr).await?;

    let io = TokioIo::new(stream);

    let (mut sender, conn) = hyper::client::conn::http1::handshake(io).await?;

    tokio::task::spawn_local(async move {
        if let Err(err) = conn.await {
            let mut stdout = io::stdout();
            stdout
                .write_all(format!("Connection failed: {:?}", err).as_bytes())
                .await
                .unwrap();
            stdout.flush().await.unwrap();
        }
    });

    let authority = url.authority().unwrap().clone();

    for secret in manifest.secrets.iter() {
        let request = Request::builder()
            .version(Version::HTTP_11)
            .method(Method::POST)
            .uri(format!("/secrets/{}", secret.name))
            .header(hyper::header::HOST, authority.as_str())
            .body(full_body("TODO"))?;

        let mut response = sender.send_request(request).await?;
        io::stdout()
            .write_all(format!("Response: {}\n", response.status()).as_bytes())
            .await
            .unwrap();
        io::stdout().flush().await.unwrap();
    }

    Ok(())
}
