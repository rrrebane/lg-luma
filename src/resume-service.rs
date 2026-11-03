//! Background work for the "continue watching" rows, off the UI thread:
//! - a listener on 127.0.0.1 that the ERR Jupiter and Lasteekraan apps post their unfinished
//!   videos to (nothing else is accepted, and it is never reachable from the network);
//! - a worker that fetches the kids recommendations and downloads card artwork, cut to size.

use crate::resume::{self, Command, IMAGE_HEIGHT, IMAGE_WIDTH, Item, Plex, Source};
use image::{
    ExtendedColorType, ImageReader, Limits, codecs::jpeg::JpegEncoder, imageops::FilterType,
};
use std::{
    io::{BufRead, BufReader, Cursor, Read, Write},
    net::{Ipv4Addr, TcpListener, TcpStream},
    path::Path,
    sync::mpsc,
    thread,
    time::Duration,
};

const MAX_BODY: usize = 128 * 1024;
const PICKS_LIMIT: usize = 4 * 1024 * 1024;
const IMAGE_LIMIT: usize = 3 * 1024 * 1024;
const PLEX_LIMIT: usize = 2 * 1024 * 1024;

pub enum Response {
    /// One app's list of unfinished videos.
    Items(Source, Vec<Item>),
    Picks(Result<Vec<Item>, String>),
    Plex(Result<Vec<Item>, String>),
    Image {
        url: String,
        ok: bool,
    },
}

pub struct Service {
    commands: mpsc::Sender<Command>,
    pub receiver: mpsc::Receiver<Response>,
}

impl Service {
    pub fn new() -> Self {
        let (commands, queue) = mpsc::channel::<Command>();
        let (responses, receiver) = mpsc::channel();

        // A second launcher (or anything else) holding the port just means no live updates.
        match TcpListener::bind((Ipv4Addr::LOCALHOST, resume::PORT)) {
            Ok(listener) => {
                let responses = responses.clone();
                thread::spawn(move || {
                    for stream in listener.incoming().flatten() {
                        if let Some(response) = serve(stream)
                            && responses.send(response).is_err()
                        {
                            break;
                        }
                    }
                });
            }
            Err(error) => eprintln!("Luma resume: port {} unavailable: {error}", resume::PORT),
        }

        // Detached on purpose: a download in flight must not delay quitting.
        thread::spawn(move || {
            let agent = ureq::Agent::config_builder()
                .timeout_global(Some(Duration::from_secs(20)))
                .max_redirects(3)
                .https_only(true)
                .proxy(None)
                .build()
                .new_agent();
            // The Plex server is on the home network and usually plain http. It only ever gets
            // requests built from the validated server address, and is never followed elsewhere.
            let lan = ureq::Agent::config_builder()
                .timeout_global(Some(Duration::from_secs(20)))
                .max_redirects(0)
                .proxy(None)
                .build()
                .new_agent();
            for command in queue {
                let response = match command {
                    Command::Picks => Response::Picks(fetch_picks(&agent)),
                    Command::Plex(plex) => Response::Plex(fetch_plex(&lan, &plex)),
                    Command::Image { url, path, token } => {
                        let ok = match token.as_deref() {
                            Some(token) => fetch_image(&lan, &url, Some(token), &path),
                            None => fetch_image(&agent, &url, None, &path),
                        }
                        .is_ok();
                        Response::Image { url, ok }
                    }
                };
                if responses.send(response).is_err() {
                    break;
                }
            }
        });
        Self { commands, receiver }
    }

    pub fn send(&self, command: Command) {
        let _ = self.commands.send(command);
    }
}

// ---------------------------------------------------------------------------------------
// Listener.

const CORS: &str = "Access-Control-Allow-Origin: *\r\nAccess-Control-Allow-Methods: POST, OPTIONS\r\nAccess-Control-Allow-Headers: content-type\r\n";

fn reply(stream: &mut TcpStream, status: &str) {
    let _ = write!(
        stream,
        "HTTP/1.1 {status}\r\n{CORS}Content-Length: 0\r\nConnection: close\r\n\r\n"
    );
}

/// Handles one connection: `POST /continue` with `{"app": "...", "items": [...]}`.
fn serve(mut stream: TcpStream) -> Option<Response> {
    stream.set_read_timeout(Some(Duration::from_secs(3))).ok()?;
    let mut reader = BufReader::new(stream.try_clone().ok()?);
    let mut line = String::new();
    reader.by_ref().take(1024).read_line(&mut line).ok()?;
    let mut parts = line.split_whitespace();
    let (method, path) = (parts.next()?.to_owned(), parts.next()?.to_owned());
    let mut length = 0usize;
    for _ in 0..64 {
        let mut header = String::new();
        reader.by_ref().take(1024).read_line(&mut header).ok()?;
        let header = header.trim_end();
        if header.is_empty() {
            break;
        }
        if let Some((name, value)) = header.split_once(':')
            && name.eq_ignore_ascii_case("content-length")
        {
            length = value.trim().parse().ok()?;
        }
    }
    if method == "OPTIONS" {
        reply(&mut stream, "204 No Content");
        return None;
    }
    if method != "POST" || path != "/continue" {
        reply(&mut stream, "404 Not Found");
        return None;
    }
    if length == 0 || length > MAX_BODY {
        reply(&mut stream, "413 Payload Too Large");
        return None;
    }
    let mut body = vec![0; length];
    reader.read_exact(&mut body).ok()?;
    let response = parse_post(&body);
    reply(
        &mut stream,
        if response.is_some() {
            "204 No Content"
        } else {
            "400 Bad Request"
        },
    );
    response
}

pub fn parse_post(body: &[u8]) -> Option<Response> {
    let value: serde_json::Value = serde_json::from_slice(body).ok()?;
    let source = Source::parse(value.get("app")?.as_str()?)?;
    Some(Response::Items(
        source,
        resume::parse_items(source, value.get("items")?),
    ))
}

// ---------------------------------------------------------------------------------------
// Downloads. Errors are fixed strings; nothing from a server is echoed.

fn download(
    agent: &ureq::Agent,
    url: &str,
    accept: &str,
    limit: usize,
    token: Option<&str>,
) -> Result<Vec<u8>, &'static str> {
    let mut request = agent
        .get(url)
        .header("Accept", accept)
        .header("User-Agent", "LumaNative");
    if let Some(token) = token {
        request = request.header("X-Plex-Token", token);
    }
    let mut response = request
        .call()
        .map_err(|_| "The server could not be reached")?;
    if !response.status().is_success() {
        return Err("The server did not accept the request");
    }
    let mut bytes = Vec::new();
    response
        .body_mut()
        .as_reader()
        .take(limit as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "The response could not be read")?;
    if bytes.len() > limit {
        return Err("The response is too large");
    }
    Ok(bytes)
}

fn fetch_picks(agent: &ureq::Agent) -> Result<Vec<Item>, String> {
    let bytes = download(
        agent,
        resume::PICKS_URL,
        "application/json",
        PICKS_LIMIT,
        None,
    )?;
    let value = serde_json::from_slice(&bytes).map_err(|_| "The page could not be read")?;
    resume::parse_picks(&value)
}

/// What Plex would continue or play next. The token goes in a header, never into the address.
fn fetch_plex(agent: &ureq::Agent, plex: &Plex) -> Result<Vec<Item>, String> {
    let url = format!("{}/library/onDeck", plex.base);
    let bytes = download(
        agent,
        &url,
        "application/json",
        PLEX_LIMIT,
        Some(&plex.token),
    )?;
    let value = serde_json::from_slice(&bytes).map_err(|_| "Plex could not be read")?;
    resume::parse_on_deck(&value, &plex.base)
}

/// Downloads an image, cuts it to the card's 16:9 and stores it as a small JPEG at `path`.
fn fetch_image(
    agent: &ureq::Agent,
    url: &str,
    token: Option<&str>,
    path: &Path,
) -> Result<(), &'static str> {
    let bytes = download(agent, url, "image/*", IMAGE_LIMIT, token)?;
    let mut reader = ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|_| "The image could not be read")?;
    let mut limits = Limits::default();
    limits.max_image_width = Some(4096);
    limits.max_image_height = Some(4096);
    limits.max_alloc = Some(64 * 1024 * 1024);
    reader.limits(limits);
    let image = reader
        .decode()
        .map_err(|_| "The image could not be decoded")?;
    let card = image
        .resize_to_fill(IMAGE_WIDTH, IMAGE_HEIGHT, FilterType::Triangle)
        .to_rgb8();
    let directory = path.parent().ok_or("No folder for the image")?;
    std::fs::create_dir_all(directory).map_err(|_| "The image folder could not be made")?;
    let mut encoded = Vec::new();
    JpegEncoder::new_with_quality(&mut encoded, 86)
        .encode(
            card.as_raw(),
            card.width(),
            card.height(),
            ExtendedColorType::Rgb8,
        )
        .map_err(|_| "The image could not be encoded")?;
    // Renamed into place so the renderer never reads half a file.
    let temporary = path.with_extension("tmp");
    std::fs::write(&temporary, encoded)
        .and_then(|()| std::fs::rename(&temporary, path))
        .map_err(|_| "The image could not be saved")
}

#[cfg(test)]
#[path = "resume-service-tests.rs"]
mod tests;
