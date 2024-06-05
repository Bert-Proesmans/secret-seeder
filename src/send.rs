pub fn client_main(
    settings: super::GlobalSettings,
    mut parser: lexopt::Parser,
) -> Result<(), Box<dyn std::error::Error>> {
    use lexopt::prelude::*;

    let mut manifest_path = None;

    while let Some(arg) = parser.next()? {
        match arg {
            Value(value) if manifest_path.is_none() => {
                manifest_path = Some(value.into());
            }
            _ => return Err(arg.unexpected())?,
        }
    }

    let manifest_path = match manifest_path {
        Some(p) => p,
        None => {
            println!("{}", super::HELP);
            return Ok(());
        }
    };

    let manifest: &'static _ = super::read_and_deserialize_manifest(manifest_path);

    // TODO Create connect object based on inputs
    run_client(&settings, manifest)
}

async fn secret_push_operation(
    secret: &super::Secret,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    use futures_util::TryStreamExt;
    use http_body_util::{BodyExt, StreamBody};
    use hyper::body::Frame;
    use hyper::Request;
    use tokio::fs::File;
    use tokio_util::io::ReaderStream;

    if cfg!(not(unix)) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::Other,
            "Must run under Unix-like platform!",
        ))?;
    }

    let unix_socket_stream = super::unix_socket::connect_unix_sock_stream().await?;
    let (mut sender, conn) = hyper::client::conn::http1::handshake(unix_socket_stream).await?;

    tokio::task::spawn(async move {
        if let Err(err) = conn.await {
            println!("Connection failed: {:?}", err);
        }
    });

    let file = File::open(&secret.source_path).await?;
    let file_length = file.metadata().await?.len();
    let file_reader = ReaderStream::new(file);
    // Convert to http_body_util::BoxBody
    let stream_body = StreamBody::new(file_reader.map_ok(Frame::data));
    let boxed_body = stream_body.boxed();

    let request = Request::post("/secrets/test")
        // Length is required by the server, otherwise it terminates our connection early
        .header(hyper::header::CONTENT_LENGTH, file_length)
        .body(boxed_body)?;

    let response = sender.send_request(request).await?;
    assert!(response.status() == hyper::StatusCode::CREATED);

    Ok(())
}

#[tokio::main(flavor = "current_thread")]
async fn run_client(
    settings: &super::GlobalSettings,
    manifest: &'static super::Manifest,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut join_set = tokio::task::JoinSet::new();
    for secret in manifest.secrets.iter() {
        join_set.spawn(secret_push_operation(secret));
    }

    while let Some(job_result) = join_set.join_next().await {
        let _ = job_result?;
    }

    Ok(())
}
