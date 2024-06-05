#[derive(Debug)]
struct CreateIOFail;
impl warp::reject::Reject for CreateIOFail {}

#[derive(Debug)]
struct WriteIOFail;
impl warp::reject::Reject for WriteIOFail {}

pub fn server_main(
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

    // TODO Create socket listener based on inputs
    run_server(&settings, manifest)
}

#[tokio::main]
async fn run_server(
    settings: &super::GlobalSettings,
    manifest: &'static super::Manifest,
) -> Result<(), Box<dyn std::error::Error>> {
    use warp::Filter;
    let manifest_tracker = std::sync::Arc::new(());

    // Wrap data for injecting into route handlers
    let state = warp::any().map(move || (manifest, manifest_tracker.clone()));

    // POST /secrets/:name  <binary data>
    let upload_route = warp::post()
        .and(warp::path("secrets"))
        .and(warp::path::param())
        .and(warp::body::content_length_limit(
            settings.max_transmission_bytes.into(),
        ))
        .and(warp::body::stream())
        .and(state)
        .and_then(handle_upload);

    let router = upload_route.recover(handle_rejection);

    if cfg!(not(unix)) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::Other,
            "Must run under Unix-like platform!",
        ))?;
    }

    let mut listener = super::unix_socket::DeleteOnDrop::bind("/tmp/warp.sock")?;
    Ok(warp::serve(router).run_incoming(&mut *listener).await)
}

async fn handle_upload(
    tag: String,
    file_body: impl futures::Stream<Item = Result<impl warp::Buf, warp::Error>> + Unpin,
    (manifest, _tracker): (&'static super::Manifest, std::sync::Arc<super::StateType>),
) -> Result<impl warp::reply::Reply, warp::reject::Rejection> {
    // TODO Verify with state
    let secret = match manifest.secrets.iter().find(|&item| item.name == tag) {
        Some(secret) => secret,
        None => return Err(warp::reject::not_found()),
    };

    // TODO Resolve physical destination node
    let target_file_path = &secret.destination_path;

    // Open the file in write mode
    let mut target_file_handle = match tokio::fs::File::create(target_file_path).await {
        Ok(f) => f,
        Err(e) => {
            eprintln!("Failed to create file: {}", e);
            return Err(warp::reject::custom(CreateIOFail));
        }
    };

    // Use StreamExt to map the stream and error to a std::io::Error, tokio::io::copy* methods
    // require the stream elements to error with std::io::Error type.
    use tokio_stream::StreamExt;
    let file_body = file_body
        .map(|result| result.map_err(|err| std::io::Error::new(std::io::ErrorKind::Other, err)));
    let mut file_body = tokio_util::io::StreamReader::new(file_body);

    // TODO Shutdown stream if receiving data takes too long
    let _bytes_written = match tokio::io::copy_buf(&mut file_body, &mut target_file_handle).await {
        Ok(b) => b,
        Err(e) => {
            eprintln!("Failed writing to file: {}", e);
            return Err(warp::reject::custom(WriteIOFail));
        }
    };

    // TODO Update state

    // TODO Signal shutdown
    // shutdown_signal.send(()).await?;

    Ok(warp::http::StatusCode::CREATED)
}

async fn handle_rejection(
    err: warp::reject::Rejection,
) -> std::result::Result<impl warp::reply::Reply, std::convert::Infallible> {
    use warp::http::StatusCode;

    let (code, message) = if err.is_not_found() {
        (StatusCode::NOT_FOUND, "Not Found".to_string())
    } else if err.find::<warp::reject::PayloadTooLarge>().is_some() {
        (StatusCode::BAD_REQUEST, "Payload too large".to_string())
    } else {
        eprintln!("unhandled error: {:?}", err);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "Internal Server Error".to_string(),
        )
    };

    Ok(warp::reply::with_status(message, code))
}
