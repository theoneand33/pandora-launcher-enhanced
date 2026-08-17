use std::error::Error;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use url::Url;

use crate::{
    constants,
    models::{FinishedAuthorization, PendingAuthorization},
};

#[derive(thiserror::Error, Debug)]
pub enum ProcessAuthorizationError {
    #[error("Unable to start http server: {0}")]
    StartServer(Box<dyn Error + Send + Sync + 'static>),
    #[error("An I/O error occurred: {0}")]
    IoError(#[from] std::io::Error),
    #[error("Server-side error: {0}")]
    ServersideError(String),
    #[error("Unable to parse HTTP request: {0}")]
    HttpParseError(#[from] httparse::Error),
    #[error("The csrf token in the request didn't match the response")]
    CsrfMismatch,
    #[error("The response didn't include the code")]
    MissingCode,
}

pub async fn start_server(
    pending_authroization: PendingAuthorization,
) -> Result<FinishedAuthorization, ProcessAuthorizationError> {
    log::info!("Starting auth redirect server on {}", constants::SERVER_ADDRESS);

    let mut listeners = Vec::new();

    match tokio::net::TcpListener::bind(constants::SERVER_ADDRESS).await {
        Ok(l) => {
            log::info!("Successfully started listening on {}", constants::SERVER_ADDRESS);
            listeners.push(l);
        },
        Err(e) => {
            log::warn!("Failed to bind {}: {}", constants::SERVER_ADDRESS, e);
        },
    }

    // On Linux, localhost often resolves to ::1 first. If we only bind 127.0.0.1,
    // a redirect to http://localhost:3160/auth can hit ::1 and get connection
    // refused. Bind ::1 as well when available.
    if constants::SERVER_ADDRESS == "127.0.0.1:3160" {
        match tokio::net::TcpListener::bind("[::1]:3160").await {
            Ok(l) => {
                log::info!("Successfully started listening on [::1]:3160");
                listeners.push(l);
            },
            Err(e) => {
                log::debug!("Failed to bind [::1]:3160: {}", e);
            },
        }
    }

    if listeners.is_empty() {
        return Err(ProcessAuthorizationError::StartServer(Box::new(std::io::Error::new(
            std::io::ErrorKind::AddrNotAvailable,
            "failed to bind auth redirect server",
        ))));
    }

    let mut buf = vec![0_u8; 1024];
    let mut read;

    loop {
        log::info!("Waiting for a new connection");
        let (mut stream, _addr) = if listeners.len() == 1 {
            listeners[0].accept().await?
        } else {
            // Wait on both listeners; Linux may deliver the redirect to either 127.0.0.1 or ::1.
            tokio::select! {
                res = listeners[0].accept() => res?,
                res = listeners[1].accept() => res?,
            }
        };
        log::info!("Got a new connection");

        read = 0;
        loop {
            let n = stream.read(&mut buf[read..]).await?;
            read += n;

            if read == buf.len() {
                log::debug!("Resizing read buffer from {} to {}", buf.len(), buf.len() * 2);
                buf.resize(buf.len() * 2, 0);
                continue;
            }

            if read == 0 {
                log::warn!("Stream immediately closed with 0 read bytes, ignoring");
                break; // Accept a new connection
            }

            let mut headers = [httparse::EMPTY_HEADER; 32];
            let mut req = httparse::Request::new(&mut headers);
            let parsed = req.parse(&buf[..read])?;

            if parsed.is_partial() {
                if n == 0 {
                    log::warn!("Only got partial request before EOF, ignoring");
                    break; // Accept a new connection
                } else {
                    continue;
                }
            }

            log::info!("Successfully received and parsed http request");

            const BAD_REQUEST_RESPONSE: &[u8] = b"HTTP/1.1 400 Bad Request\r\nContent-Length: 0\r\n\r\n";
            const NOT_FOUND_RESPONSE: &[u8] = b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n";

            if req.method != Some("GET") || req.path.is_none() {
                log::warn!("404: unexpected method or missing path: {:?}", req.path);
                stream.write_all(NOT_FOUND_RESPONSE).await?;
                break;
            }
            let raw_path = req.path.unwrap();

            // Browsers normally send origin-form "GET /auth?code=... HTTP/1.1".
            // With a proxy they can send absolute-form "GET http://localhost:3160/auth?code=... HTTP/1.1".
            let url_str = if raw_path.starts_with("http://") || raw_path.starts_with("https://") {
                raw_path.to_string()
            } else {
                format!("{}{}", constants::REDIRECT_URL_BASE, raw_path)
            };

            let Ok(url) = Url::parse(&url_str) else {
                log::warn!("400: failed to parse URL from path {:?}", raw_path);
                stream.write_all(BAD_REQUEST_RESPONSE).await?;
                break;
            };

            // Allow "/auth" and "/auth/" (some browsers or proxies normalize the slash).
            let path_no_slash = url.path().trim_end_matches('/');
            if path_no_slash != "/auth" {
                log::warn!("404: unexpected path {:?} (raw {:?})", url.path(), raw_path);
                stream.write_all(NOT_FOUND_RESPONSE).await?;
                break;
            }

            let mut error = None;
            let mut error_description = None;
            let mut code = None;
            let mut state = None;

            for (key, value) in url.query_pairs() {
                match &*key {
                    "error" => error = Some(value),
                    "error_description" => error_description = Some(value),
                    "code" => code = Some(value),
                    "state" => state = Some(value),
                    _ => {
                        log::warn!("Unknown parameter: {:?} => {:?}", key, value);
                    },
                }
            }

            if let Some(error) = error {
                let full_error = if let Some(error_description) = error_description {
                    let response =
                        create_response(&format!("An error occurred: {}", &*error), &error_description, true);
                    stream.write_all(response.as_bytes()).await?;
                    format!("An error occurred: {}\n{}", error, error_description)
                } else {
                    let response = create_response(&format!("An error occurred: {}", &*error), "", true);
                    stream.write_all(response.as_bytes()).await?;
                    format!("An error occurred: {}", error)
                };
                return Err(ProcessAuthorizationError::ServersideError(full_error));
            }

            if let Some(state) = state
                && &*state != pending_authroization.csrf_token.secret()
            {
                let response = create_response(
                    "Error: CSRF Mismatch!",
                    "Did you reload the tab instead of going through the proper authorization flow?",
                    true,
                );
                stream.write_all(response.as_bytes()).await?;
                return Err(ProcessAuthorizationError::CsrfMismatch);
            }

            let Some(code) = code else {
                let response = create_response("Error", "Missing required 'code' parameter", true);
                stream.write_all(response.as_bytes()).await?;
                return Err(ProcessAuthorizationError::MissingCode);
            };

            let response = create_response("Authorization complete", "You may now close this window", false);
            stream.write_all(response.as_bytes()).await?;

            return Ok(FinishedAuthorization {
                pending: pending_authroization,
                code: code.to_string(),
            });
        }
    }
}

fn create_response(main: &str, secondary: &str, error: bool) -> String {
    let status = if error { "400 Bad Request" } else { "200 OK" };

    let body = format!(include_str!("auth_page.html"), main, secondary);
    let body_length = body.len();

    format!(
        "HTTP/1.1 {status}\r\nContent-Type: text/html; charset=UTF-8\r\nContent-Length: {}\r\n\r\n{}",
        body_length, body
    )
}
