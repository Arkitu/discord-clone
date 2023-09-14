extern crate time;
extern crate argon2;

use std::{path::PathBuf, sync::Arc, borrow::Borrow, fs::read_to_string, collections::HashMap};
use db::DB;
use tokio::{net::TcpListener, io::{AsyncWriteExt, AsyncWrite}};
use minijinja::{Environment, context, value::StructObject};
use http_bytes::{http, http::StatusCode};
use walkdir::WalkDir;
use anyhow::Error;

mod api;
mod db;

const HOST: &str = "127.0.0.1:8080";

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let argv = std::env::args().collect::<Vec<_>>();
    let dev_mode = argv.contains(&"--dev".to_string());
    
    let env = Arc::new(create_env().await);

    let db_path = if dev_mode {
        Some("test.db")
    } else {
        None
    };

    let db = Arc::new(DB::new(db_path).await);

    let tcp = TcpListener::bind(HOST).await?;

    loop {
        match tcp.accept().await {
            Err(e) => eprintln!("Error accepting connection: {}", e),
            Ok((stream, addr)) => {
                println!("Got connection from: {}", addr);
                let env = if dev_mode {
                    // Reload environment
                    Arc::new(create_env().await)
                } else {
                    env.clone()
                };
                let db = db.clone();
                let dev_mode = dev_mode.clone();
                tokio::spawn(async move {
                    stream.readable().await.unwrap();
                    let mut buffer = [0; 1024];
                    stream.try_read(&mut buffer).unwrap();

                    let mut headers = [httparse::EMPTY_HEADER; 64];
                    let mut header_req = httparse::Request::new(&mut headers);
                    if let Err(e) = header_req.parse(buffer.as_ref()) {
                        eprintln!("Error parsing request: {}", e);
                        let res = http_bytes::Response::builder()
                            .status(StatusCode::BAD_REQUEST)
                            .body(())
                            .unwrap();
                        if let Err(e) = write_empty_response(res, stream).await {
                            eprintln!("Error writing response: {}", e);
                        };
                        return;
                    }

                    let body = match std::str::from_utf8(&buffer) {
                        Err(_) => None,
                        Ok(b) => match b.split_once("\r\n\r\n") {
                            None => None,
                            Some((_, b)) => Some(b)
                        }
                    };

                    let req = http::request::Builder::new()
                        .method(header_req.method.unwrap())
                        .uri(header_req.path.unwrap())
                        .version(http::Version::HTTP_11)
                        .body(body.map(|b| b.to_string()))
                        .unwrap();

                    if dev_mode && req.uri().path() == "/debug" {
                        println!("{:#?}", req);
                        println!("{:#?}", std::str::from_utf8(&buffer));
                        return
                    }

                    match handle_connection(req, env, db, dev_mode).await {
                        Ok(Some(res)) => {
                            if let Err(e) = write_response(res, stream).await {
                                eprintln!("Error writing response: {}", e);
                            }
                        },
                        Ok(None) => {},
                        Err(HandleError::InternalServerError(e)) => {
                            eprintln!("Internal server error: {}", e);
                            let res = http_bytes::Response::builder()
                                .status(StatusCode::INTERNAL_SERVER_ERROR)
                                .body(b"Internal server error".to_vec())
                                .unwrap();
                            if let Err(e) = write_response(res, stream).await {
                                eprintln!("Error writing response: {}", e);
                            };
                        },
                        Err(HandleError::BadRequest) => {
                            let res = http_bytes::Response::builder()
                                .status(StatusCode::BAD_REQUEST)
                                .body(b"Bad request".to_vec())
                                .unwrap();
                            if let Err(e) = write_response(res, stream).await {
                                eprintln!("Error writing response: {}", e);
                            };
                        },
                        Err(HandleError::NotFound) => {
                            let res = http_bytes::Response::builder()
                                .status(StatusCode::NOT_FOUND)
                                .body(b"Not found".to_vec())
                                .unwrap();
                            if let Err(e) = write_response(res, stream).await {
                                eprintln!("Error writing response: {}", e);
                            };
                        }
                    }
                });
            }
        }
    }
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

enum HandleError {
    InternalServerError(Error),
    BadRequest,
    NotFound
}
impl From<Error> for HandleError {
    fn from(e: Error) -> Self {
        Self::InternalServerError(e)
    }
}

struct HttpArgs(HashMap<String, String>);
impl HttpArgs {
    pub fn new() -> Self {
        Self(HashMap::new())
    }
    pub fn insert(&mut self, k: String, v: String) {
        self.0.insert(k, v);
    }
}
impl StructObject for HttpArgs {
    fn get_field(&self, name: &str) -> Option<minijinja::Value> {
        match self.0.get(name) {
            None => None,
            Some(v) => Some(minijinja::Value::from(v.clone()))
        }
    }
}

async fn handle_connection<'a>(req: http::Request<Option<String>>, env: Arc<Environment<'_>>, db: Arc<DB>, dev_mode: bool) -> Result<Option<http::Response<Vec<u8>>>, HandleError> {
    let mut path = req.uri().path();
    println!("Got request for: {}", path);
    if path == "/" {
        path = "/index.html";
    }

    let mut args_str = "";
    if let Some((p, a)) = path.split_once('?') {
        path = p;
        args_str = a;
    }

    let mut args = HttpArgs::new();
    for arg in args_str.split('&') {
        if let Some((name, value)) = arg.split_once('=') {
            args.insert(name.into(), url_escape::decode(value).into_owned());
        }
    }

    if path.starts_with("/api/") {
        return handle_api(req, path, args, db).await;
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
                            return Err(HandleError::NotFound);
                        } else {
                            return Err(HandleError::InternalServerError(e.into()));
                        }
                    }
                }
            } else {
                return Err(HandleError::InternalServerError(e.into()));
            }
        }
    };

    let res = match template.render(minijinja::Value::from_struct_object(args)) {
        Ok(b) => b,
        Err(e) => return Err(HandleError::InternalServerError(e.into()))
    };
    let res = res.as_bytes();

    let res = http::Response::builder()
        .body(res.to_owned())
        .unwrap();

    Ok(Some(res))
}

async fn handle_api(req: http::Request<Option<String>>, path: &str, args: HttpArgs, db: Arc<DB>) -> Result<Option<http::Response<Vec<u8>>>, HandleError> {
    // split the "/api/"
    let path = path[5..].to_string();
    let mut path = path.split('/');

    match path.next() {
        Some("create_class") => {
            if req.method() != http::Method::PUT {
                return Err(HandleError::BadRequest);
            }

            let name = match args.0.get("name") {
                None => return Err(HandleError::BadRequest),
                Some(n) => n.clone()
            };

            db.insert_class(name).await?;
        },
        Some("create_user") => {
            if req.method() != http::Method::PUT {
                return Err(HandleError::BadRequest);
            }

            let name = match args.0.get("name") {
                None => return Err(HandleError::BadRequest),
                Some(n) => n.clone()
            };
            let password = match args.0.get("password") {
                None => return Err(HandleError::BadRequest),
                Some(n) => n.clone()
            };
            let password_hash = argon2::hash_encoded(password, salt, config)
        }
        None | Some(_) => return Err(HandleError::BadRequest)
    }

    Ok(None)
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
