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


#[derive(Debug)]
struct CreateIOFail;
impl warp::reject::Reject for CreateIOFail {}

#[derive(Debug)]
struct WriteIOFail;
impl warp::reject::Reject for WriteIOFail {}

#[cfg(not(unix))]
#[tokio::main]
async fn main() {
    panic!("Must run under Unix-like platform!");
}

#[cfg(unix)]
#[tokio::main]
async fn main() {
    use tokio::net::UnixListener;
    use tokio_stream::wrappers::UnixListenerStream;
    use warp::Filter;

    pretty_env_logger::init();

    // TODO parametrize max request body
    let config_max_body_size = 1024 * 16;

    let manifest: &'static _ = read_and_deserialize_manifest();
    let manifest_tracker = std::sync::Arc::new(());

    let listener = UnixListener::bind("/tmp/warp.sock").unwrap();
    let incoming = UnixListenerStream::new(listener);

    // Wrap data for injecting into route handlers
    let state = warp::any().map(move || (manifest, manifest_tracker.clone()));

    // POST /secrets/:name  <binary data>
    let upload_route = warp::post()
        .and(warp::path("secrets"))
        .and(warp::path::param())
        .and(warp::body::content_length_limit(config_max_body_size))
        .and(warp::body::stream())
        .and(state)
        .and_then(upload);

    let router = upload_route.recover(handle_rejection);
    warp::serve(router).run_incoming(incoming).await;
}

fn read_and_deserialize_manifest() -> &'static mut Manifest {
    let example_manifest = Manifest {
        secrets: vec![Secret {
            name: "test".to_string(),
            source_path: std::path::PathBuf::from(r"/tmp/source"),
            destination_path: std::path::PathBuf::from(r"/tmp/source"),
            owner: "bert-proesmans".to_string(),
            group: "bert-proesmans".to_string(),
            mode: "0664".to_string(),
        }],
    };

    Box::leak(Box::new(example_manifest))
}

async fn upload(
    tag: String,
    file_body: impl futures::Stream<Item = Result<impl warp::Buf, warp::Error>> + Unpin,
    (manifest, _tracker): (&'static Manifest, std::sync::Arc<StateType>),
) -> Result<impl warp::reply::Reply, warp::reject::Rejection> {
    
    // TODO Verify with state
    let secret = match manifest.secrets.iter().find(|&item| item.name == tag) {
        Some(secret) => secret,
        None => return Err(warp::reject::not_found())
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
    let file_body = file_body.map(|result| result.map_err(|err| {
        std::io::Error::new(std::io::ErrorKind::Other, err)
    }));
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
