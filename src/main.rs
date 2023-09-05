extern crate time;

use std::{path::PathBuf, sync::Arc, borrow::Borrow, fs::read_to_string};
use tokio::{net::TcpListener, io::{AsyncWriteExt, AsyncWrite}};
use minijinja::{Environment, context};
use http_bytes::{http, http::StatusCode};
use walkdir::WalkDir;
use anyhow::Result;

mod api;

const HOST: &str = "127.0.0.1:8080";

#[tokio::main]
async fn main() -> Result<()> {
    let argv = std::env::args().collect::<Vec<_>>();
    let dev_mode = argv.contains(&"--dev".to_string());
    
    let env = Arc::new(create_env().await);

    let tcp = TcpListener::bind(HOST).await?;

    println!("Initializing API...");
    let mut api = api::APIClient::new();
    println!("API initialized");
    api.connect().await?;

    // loop {
    //     match tcp.accept().await {
    //         Err(e) => eprintln!("Error accepting connection: {}", e),
    //         Ok((stream, addr)) => {
    //             println!("Got connection from: {}", addr);
    //             let env = if dev_mode {
    //                 // Reload environment
    //                 Arc::new(create_env().await)
    //             } else {
    //                 env.clone()
    //             };
    //             let api = api.clone();
    //             tokio::spawn(async move {
    //                 stream.readable().await.unwrap();
    //                 let mut buffer = [0; 1024];
    //                 stream.try_read(&mut buffer).unwrap();

    //                 let mut headers = [httparse::EMPTY_HEADER; 64];
    //                 let mut req = httparse::Request::new(&mut headers);
    //                 if let Err(e) = req.parse(buffer.as_ref()) {
    //                     eprintln!("Error parsing request: {}", e);
    //                     let res = http_bytes::Response::builder()
    //                         .status(StatusCode::BAD_REQUEST)
    //                         .body(())
    //                         .unwrap();
    //                     if let Err(e) = write_empty_response(res, stream).await {
    //                         eprintln!("Error writing response: {}", e);
    //                     };
    //                     return;
    //                 }

    //                 if let Some(res) = handle_connection(req, env, api).await {
    //                     if let Err(e) = write_response(res, stream).await {
    //                         eprintln!("Error writing response: {}", e);
    //                     }
    //                 }
    //             });
    //         }
    //     }
    // }
    Ok(())
}

async fn create_env() -> Environment<'static> {
    let mut env = Environment::new();
    let template_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("public");
    println!("Getting templates from: {}", template_path.display());
    for entry in WalkDir::new(&template_path) {
        let entry = entry.unwrap();
        if entry.file_type().is_file() {
            let path = entry.path();
            if path.file_name().unwrap() == ".DS_Store" {
                continue;
            }
            let path = path.strip_prefix(&template_path).unwrap();
            let mut path = "/".to_string() + path.to_str().unwrap();
            if path.ends_with(".j2") {
                path = path[..path.len() - 3].to_string();
            }
            println!("Loading template: {}", path);
            let content = read_to_string(entry.path()).unwrap();
            env.add_template_owned(path.to_owned(), content).unwrap();
        }
    }
    env
}

async fn handle_connection<'a>(req: httparse::Request<'_, '_>, env: Arc<Environment<'_>>, api: api::APIClient) -> Option<http::Response<Vec<u8>>> {
    let mut path = req.path.unwrap_or("/");
    println!("Got request for: {}", path);
    if path == "/" {
        path = "/index.html";
    }

    let template = match env.get_template(path) {
        Ok(t) => t,
        Err(e) => {
            if let minijinja::ErrorKind::TemplateNotFound = e.kind() {
                // Try various extensions
                match env.get_template(&(path.to_string() + ".html")) {
                    Ok(t) => t,
                    Err(e) => {
                        // TODO: Check if it's a directory and serve index.html
                        if let minijinja::ErrorKind::TemplateNotFound = e.kind() {
                            println!("Template not found: {}", path);
                            let res = http::Response::builder()
                                .status(StatusCode::NOT_FOUND)
                                .body((*b"<h1>Not found</h1>").into())
                                .unwrap();
                            return Some(res);
                        } else {
                            eprintln!("Error getting template: {}", e);
                            let res = http::Response::builder()
                                .status(StatusCode::INTERNAL_SERVER_ERROR)
                                .body((*b"<h1>Internal server error</h1>").into())
                                .unwrap();
                            return Some(res);
                        }
                    }
                }
            } else {
                eprintln!("Error getting template: {}", e);
                let res = http::Response::builder()
                    .status(StatusCode::INTERNAL_SERVER_ERROR)
                    .body((*b"<h1>Internal server error</h1>").into())
                    .unwrap();
                return Some(res);
            }
        }
    };

    let res = match template.render(context! {}) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("Error rendering template: {}", e);
            let res = http::Response::builder()
                .status(StatusCode::INTERNAL_SERVER_ERROR)
                .body((*b"<h1>Internal server error</h1>").into())
                .unwrap();
            return Some(res);
        }
    };
    let res = res.as_bytes();

    let res = http::Response::builder()
        .body(res.to_owned())
        .unwrap();

    Some(res)
}

/// Code from this : https://docs.rs/simple-server/latest/src/simple_server/lib.rs.html#1-495
/// but modified for tokio
async fn write_response<'a, S: AsyncWrite + Unpin>(
    response: http::Response<Vec<u8>>,
    mut stream: S,
) -> std::io::Result<()> {
    use std::fmt::Write;

    let (parts, body) = response.into_parts();
    let body: &[u8] = body.borrow();

    let mut text = format!(
        "HTTP/1.1 {} {}\r\n",
        parts.status.as_str(),
        parts
            .status
            .canonical_reason()
            .expect("Unsupported HTTP Status"),
    );

    if !parts.headers.contains_key(http::header::DATE) {
        // "%a, %d %b %Y %H:%M:%S GMT"
        let date = time::strftime("%a, %d %b %Y %H:%M:%S GMT", &time::now_utc()).unwrap();
        write!(text, "date: {}\r\n", date).unwrap();
    }
    if !parts.headers.contains_key(http::header::CONNECTION) {
        write!(text, "connection: close\r\n").unwrap();
    }
    if !parts.headers.contains_key(http::header::CONTENT_LENGTH) {
        write!(text, "content-length: {}\r\n", body.len()).unwrap();
    }
    for (k, v) in parts.headers.iter() {
        write!(text, "{}: {}\r\n", k.as_str(), v.to_str().unwrap()).unwrap();
    }

    write!(text, "\r\n").unwrap();

    stream.write(text.as_bytes()).await?;
    stream.write(body).await?;
    stream.flush().await?;
    Ok(())
}

async fn write_empty_response<S: AsyncWrite + Unpin>(
    response: http::Response<()>,
    mut stream: S,
) -> std::io::Result<()> {
    use std::fmt::Write;

    let (parts, _) = response.into_parts();

    let mut text = format!(
        "HTTP/1.1 {} {}\r\n",
        parts.status.as_str(),
        parts
            .status
            .canonical_reason()
            .expect("Unsupported HTTP Status"),
    );

    if !parts.headers.contains_key(http::header::DATE) {
        // "%a, %d %b %Y %H:%M:%S GMT"
        let date = time::strftime("%a, %d %b %Y %H:%M:%S GMT", &time::now_utc()).unwrap();
        write!(text, "date: {}\r\n", date).unwrap();
    }
    if !parts.headers.contains_key(http::header::CONNECTION) {
        write!(text, "connection: close\r\n").unwrap();
    }
    for (k, v) in parts.headers.iter() {
        write!(text, "{}: {}\r\n", k.as_str(), v.to_str().unwrap()).unwrap();
    }

    write!(text, "\r\n").unwrap();

    stream.write(text.as_bytes()).await?;
    stream.flush().await?;
    Ok(())
}