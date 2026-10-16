//! The dev server `nomi run` gives a browser program, as `elm reactor`
//! serves a project: its built code, its entry's static files, and a page
//! for every other path without an extension, so an application's own
//! routes load it, all on this machine alone. Each page listens for an
//! event telling it to reload. There is no proxy: a page calls an API on
//! another origin.

use std::fmt;
use std::fs::{self, File};
use std::io::{self, Write};
use std::net::{Ipv4Addr, TcpListener};
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread::{self, JoinHandle};

use percent_encoding::percent_decode_str;
use tiny_http::{Header, Method, Request, Response, ResponseBox, StatusCode};

/// What a server serves.
#[derive(Debug)]
pub struct Site {
    /// The built code, absolute, served at `/` followed by its file name.
    pub code: PathBuf,
    /// The module whose `main` the generated page starts.
    pub module: String,
    /// The generated page's title: its project's name.
    pub title: String,
    /// The directory of static files, canonical.
    pub static_dir: Option<PathBuf>,
}

/// Where a page listens for reloads.
const EVENTS: &str = "/__nomi/events";

/// What every page is given before `</body>`: a listener that reloads it
/// when the server says so.
const RELOAD: &str = "<script>new EventSource(\"/__nomi/events\").addEventListener(\"reload\", \
                      function () { location.reload(); });</script>\n";

/// The open event streams, written to on a reload.
type Streams = Arc<Mutex<Vec<Box<dyn Write + Send>>>>;

/// A dev server running on a thread of its own until dropped.
pub struct Server {
    http: Arc<tiny_http::Server>,
    port: u16,
    streams: Streams,
    stopping: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl fmt::Debug for Server {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Server").field("port", &self.port).finish()
    }
}

impl Server {
    /// Serves `site` on `port` of 127.0.0.1, or any free one for 0.
    pub fn start(site: Site, port: u16) -> io::Result<Server> {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, port))?;
        let port = listener.local_addr()?.port();
        let http = tiny_http::Server::from_listener(listener, None).map_err(io::Error::other)?;
        let http = Arc::new(http);
        let streams = Streams::default();
        let stopping = Arc::new(AtomicBool::new(false));
        let thread = {
            let (http, streams, stopping) = (http.clone(), streams.clone(), stopping.clone());
            thread::spawn(move || {
                loop {
                    match http.recv() {
                        Ok(request) => answer(&site, &streams, request),
                        Err(_) if stopping.load(Ordering::Relaxed) => break,
                        // A connection that failed to open.
                        Err(_) => {}
                    }
                }
            })
        };
        Ok(Server {
            http,
            port,
            streams,
            stopping,
            thread: Some(thread),
        })
    }

    /// The port it listens on.
    pub fn port(&self) -> u16 {
        self.port
    }

    /// Tells every page open to reload, forgetting those since closed.
    pub fn reload(&self) {
        let mut streams = self.streams.lock().unwrap_or_else(PoisonError::into_inner);
        streams.retain_mut(|stream| {
            (stream.write_all(b"event: reload\ndata: reload\n\n"))
                .and_then(|()| stream.flush())
                .is_ok()
        });
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.stopping.store(true, Ordering::Relaxed);
        self.http.unblock();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
        (self.streams.lock().unwrap_or_else(PoisonError::into_inner)).clear();
    }
}

/// Answers `request` from `site`; an event stream joins `streams`.
fn answer(site: &Site, streams: &Streams, request: Request) {
    let (method, asked) = (request.method(), asked(request.url()));
    if !matches!(method, Method::Get | Method::Head) {
        let allow = header("Allow", "GET, HEAD");
        let _ = request.respond(Response::empty(405).with_header(allow));
        return;
    }
    let found = match asked {
        Asked::Events if *method == Method::Get => return listen(streams, request),
        Asked::Events => Ok(Found::Events),
        Asked::File(path) => found(site, &path),
        Asked::Refused => Err(StatusCode(404)),
    };
    let _ = match found.and_then(|found| response(site, found)) {
        Ok(response) => request.respond(response),
        Err(status) => request.respond(Response::empty(status)),
    };
}

/// Starts an event stream on `request`'s connection and keeps it, kept
/// before its head is sent, so a page that has the head misses no reload.
fn listen(streams: &Streams, request: Request) {
    let mut streams = streams.lock().unwrap_or_else(PoisonError::into_inner);
    let mut stream = request.into_writer();
    let head = "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\n\
                Cache-Control: no-store\r\n\r\n";
    if (stream.write_all(head.as_bytes()))
        .and_then(|()| stream.flush())
        .is_ok()
    {
        streams.push(stream);
    }
}

/// What a request's target asks for.
#[derive(Debug, PartialEq, Eq)]
enum Asked {
    /// The event stream of reloads.
    Events,
    /// A file, by its path under the site, of normal components only.
    File(PathBuf),
    /// Nothing the site may give: a target that leaves it or isn't a
    /// path.
    Refused,
}

/// What `target`, a request's, asks for: its path, decoded, without
/// its query; refused if any segment is `..` or isn't a plain name.
fn asked(target: &str) -> Asked {
    let path = target.split(['?', '#']).next().unwrap_or_default();
    let Some(path) = path.strip_prefix('/') else {
        return Asked::Refused;
    };
    let Ok(path) = percent_decode_str(path).decode_utf8() else {
        return Asked::Refused;
    };
    if path == EVENTS[1..] {
        return Asked::Events;
    }
    let mut file = PathBuf::new();
    for segment in path.split('/').filter(|s| !s.is_empty() && *s != ".") {
        let mut parts = Path::new(segment).components();
        let plain = !segment.contains(['\\', '\0'])
            && matches!(
                (parts.next(), parts.next()),
                (Some(Component::Normal(_)), None)
            );
        if !plain {
            return Asked::Refused;
        }
        file.push(segment);
    }
    Asked::File(file)
}

/// What a site gives for a path under it.
#[derive(Debug)]
enum Found {
    /// The head of the event stream, for HEAD.
    Events,
    /// A file, canonical, as it is.
    File(PathBuf),
    /// A page, from this file, or generated where none.
    Page(Option<PathBuf>),
}

/// What `site` gives for `path`: the code at its file name, a static
/// file, or the page for a path without an extension.
fn found(site: &Site, path: &Path) -> Result<Found, StatusCode> {
    if site
        .code
        .file_name()
        .is_some_and(|name| path == Path::new(name))
    {
        return Ok(Found::File(site.code.clone()));
    }
    let static_file = |path: &Path| {
        let dir = site.static_dir.as_ref()?;
        let file = fs::canonicalize(dir.join(path)).ok()?;
        // A link may lead out of the directory.
        (file.starts_with(dir) && file.is_file()).then_some(file)
    };
    if let Some(file) = static_file(path) {
        return Ok(Found::File(file));
    }
    if path.extension().is_some() {
        return Err(StatusCode(404));
    }
    let index = static_file(Path::new("index.html"));
    let html = has_extension(&site.code, &["html", "htm"]);
    Ok(Found::Page(
        index.or_else(|| html.then(|| site.code.clone())),
    ))
}

/// The response giving `found`, its HTML with [`RELOAD`].
fn response(site: &Site, found: Found) -> Result<ResponseBox, StatusCode> {
    const HTML: &str = "text/html; charset=utf-8";
    let page = |page: &str| Response::from_data(with_reload(page)).boxed();
    let read = |path: &Path| fs::read(path).map_err(|_| StatusCode(404));
    let (response, content_type) = match found {
        Found::Events => (Response::empty(200).boxed(), "text/event-stream"),
        Found::File(path) if !has_extension(&path, &["html", "htm"]) => {
            let file = File::open(&path).map_err(|_| StatusCode(404))?;
            (Response::from_file(file).boxed(), content_type(&path))
        }
        Found::File(path) | Found::Page(Some(path)) => {
            (page(&String::from_utf8_lossy(&read(&path)?)), HTML)
        }
        Found::Page(None) => (page(&generated(site)), HTML),
    };
    Ok(response
        .with_header(header("Content-Type", content_type))
        .with_header(header("Cache-Control", "no-store")))
}

/// The page that starts `site`'s program where it has none of its own,
/// as `elm make` writes one around its code, importing `Elm` where the
/// code is an ES module.
fn generated(site: &Site) -> String {
    let script = site.code.file_name().unwrap_or_default().to_string_lossy();
    let module = &site.module;
    let init =
        format!("var app = Elm.{module}.init({{ node: document.getElementById(\"elm\") }});");
    let scripts = if has_extension(&site.code, &["mjs"]) {
        format!("<script type=\"module\">\nimport {{ Elm }} from \"/{script}\";\n{init}\n</script>")
    } else {
        format!("<script src=\"/{script}\"></script>\n<script>\n{init}\n</script>")
    };
    let title = escaped(&site.title);
    format!(
        "<!DOCTYPE HTML>\n<html>\n<head>\n  <meta charset=\"UTF-8\">\n  <title>{title}</title>\n  \
         <style>body {{ padding: 0; margin: 0; }}</style>\n</head>\n\n<body>\n\n\
         <pre id=\"elm\"></pre>\n\n{scripts}\n\n</body>\n</html>"
    )
}

/// `text` as HTML's text.
fn escaped(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// `page` with [`RELOAD`] before its last `</body>`, or at its end.
fn with_reload(page: &str) -> Vec<u8> {
    let at = page
        .to_ascii_lowercase()
        .rfind("</body>")
        .unwrap_or(page.len());
    [&page[..at], RELOAD, &page[at..]].concat().into_bytes()
}

/// Whether `path`'s extension is one of `extensions`, in any case.
fn has_extension(path: &Path, extensions: &[&str]) -> bool {
    let extension = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or_default();
    extensions.iter().any(|e| e.eq_ignore_ascii_case(extension))
}

/// The media type of the file at `path`, by its extension.
fn content_type(path: &Path) -> &'static str {
    let extension = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or_default();
    let types = [
        ("css", "text/css; charset=utf-8"),
        ("js", "text/javascript; charset=utf-8"),
        ("mjs", "text/javascript; charset=utf-8"),
        ("json", "application/json"),
        ("map", "application/json"),
        ("txt", "text/plain; charset=utf-8"),
        ("xml", "application/xml"),
        ("svg", "image/svg+xml"),
        ("png", "image/png"),
        ("jpg", "image/jpeg"),
        ("jpeg", "image/jpeg"),
        ("gif", "image/gif"),
        ("webp", "image/webp"),
        ("avif", "image/avif"),
        ("ico", "image/x-icon"),
        ("woff", "font/woff"),
        ("woff2", "font/woff2"),
        ("ttf", "font/ttf"),
        ("otf", "font/otf"),
        ("wasm", "application/wasm"),
        ("pdf", "application/pdf"),
        ("mp3", "audio/mpeg"),
        ("wav", "audio/wav"),
        ("mp4", "video/mp4"),
        ("webm", "video/webm"),
        ("webmanifest", "application/manifest+json"),
    ];
    (types.iter())
        .find(|(e, _)| e.eq_ignore_ascii_case(extension))
        .map_or("application/octet-stream", |&(_, t)| t)
}

fn header(name: &str, value: &str) -> Header {
    Header::from_bytes(name, value).expect("a header's name and value are ASCII")
}

#[cfg(test)]
mod tests {
    use std::net::TcpListener;
    use std::time::Duration;

    use nomi_testkit::http::{events, request};

    use super::*;

    /// A site in a new directory: `Main.js` built beside `public`, its
    /// static files, `secret.txt` beside them, and in `public` `style.css`
    /// and links out to the secret and the directory around it; with an
    /// `index.html` of its own where `index`.
    fn site(index: bool) -> (tempfile::TempDir, Site) {
        let work = tempfile::tempdir().unwrap();
        let dir = fs::canonicalize(work.path()).unwrap();
        let public = dir.join("public");
        fs::create_dir_all(public.join("css")).unwrap();
        fs::write(dir.join("Main.js"), "var Elm = {};\n").unwrap();
        fs::write(dir.join("secret.txt"), "secret\n").unwrap();
        fs::write(public.join("css/style.css"), "body {}\n").unwrap();
        if index {
            fs::write(
                public.join("index.html"),
                "<html><BODY><script src=\"/Main.js\"></script></BODY></html>\n",
            )
            .unwrap();
        }
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(dir.join("secret.txt"), public.join("leak.txt")).unwrap();
            std::os::unix::fs::symlink(&dir, public.join("up")).unwrap();
        }
        let site = Site {
            code: dir.join("Main.js"),
            module: "Pages.Home".to_owned(),
            title: "web".to_owned(),
            static_dir: Some(public),
        };
        (work, site)
    }

    #[test]
    fn a_target_is_a_file_under_the_site_or_refused() {
        let file = |path: &str| Asked::File(PathBuf::from(path));
        let cases = [
            ("/", file("")),
            ("/css/style.css?v=1#top", file("css/style.css")),
            ("/./a//b/", file("a/b")),
            ("/a%20b", file("a b")),
            ("/__nomi/events", Asked::Events),
            ("/../x", Asked::Refused),
            ("/a/../../x", Asked::Refused),
            ("/%2e%2e/x", Asked::Refused),
            ("/%2E%2E%2Fx", Asked::Refused),
            ("/a%5c..%5cx", Asked::Refused),
            ("/a%00", Asked::Refused),
            ("/%ff", Asked::Refused),
            ("http://example.com/x", Asked::Refused),
            ("*", Asked::Refused),
        ];
        for (target, wanted) in cases {
            assert_eq!(asked(target), wanted, "{target}");
        }
    }

    #[test]
    fn it_serves_the_code_the_static_files_and_a_page_for_every_route() {
        let (_work, site) = site(false);
        let server = Server::start(site, 0).unwrap();
        let get = |target| request(server.port(), "GET", target).unwrap();
        let page = get("/");
        assert_eq!(page.status, 200);
        assert_eq!(
            page.header("content-type"),
            Some("text/html; charset=utf-8")
        );
        let page = page.text();
        assert!(
            page.contains("<script src=\"/Main.js\"></script>"),
            "{page}"
        );
        assert!(page.contains("Elm.Pages.Home.init("), "{page}");
        assert!(page.contains("<title>web</title>"), "{page}");
        assert!(page.contains(&format!("{RELOAD}</body>")), "{page}");
        // An application's own routes get the page too.
        assert_eq!(get("/todos/3").text(), page);
        assert_eq!(get("/Main.js").text(), "var Elm = {};\n");
        assert_eq!(
            get("/Main.js").header("content-type"),
            Some("text/javascript; charset=utf-8")
        );
        let style = get("/css/style.css");
        assert_eq!((style.status, style.text()), (200, "body {}\n".to_owned()));
        assert_eq!(
            style.header("content-type"),
            Some("text/css; charset=utf-8")
        );
        assert_eq!(get("/missing.png").status, 404);
        let head = request(server.port(), "HEAD", "/css/style.css").unwrap();
        assert_eq!(head.status, 200);
        assert_eq!(head.header("content-length"), Some("8"));
        assert!(head.body.is_empty());
        let post = request(server.port(), "POST", "/").unwrap();
        assert_eq!(post.status, 405);
        assert_eq!(post.header("allow"), Some("GET, HEAD"));
    }

    #[test]
    fn the_page_imports_an_es_module() {
        let site = Site {
            code: PathBuf::from("/w/Main.mjs"),
            module: "Main".to_owned(),
            title: "w".to_owned(),
            static_dir: None,
        };
        let page = generated(&site);
        assert!(page.contains("<script type=\"module\">\nimport { Elm } from \"/Main.mjs\";\nvar app = Elm.Main.init("), "{page}");
    }

    #[test]
    fn a_static_index_is_the_page_with_the_reload_before_its_body_ends() {
        let (_work, site) = site(true);
        let server = Server::start(site, 0).unwrap();
        let page = request(server.port(), "GET", "/some/route").unwrap();
        assert_eq!(
            page.text(),
            format!("<html><BODY><script src=\"/Main.js\"></script>{RELOAD}</BODY></html>\n")
        );
    }

    #[test]
    fn nothing_outside_the_static_directory_is_served() {
        let (_work, site) = site(false);
        let server = Server::start(site, 0).unwrap();
        for target in [
            "/../secret.txt",
            "/css/../../secret.txt",
            "/%2e%2e/secret.txt",
            "/..%2fsecret.txt",
            "/leak.txt",
            "/up/secret.txt",
        ] {
            let response = request(server.port(), "GET", target).unwrap();
            assert_eq!(response.status, 404, "{target}");
        }
        // An absolute path is a route like any other, not a file.
        let absolute = request(server.port(), "GET", "//etc/hostname").unwrap();
        assert!(
            absolute.text().starts_with("<!DOCTYPE HTML>"),
            "{absolute:?}"
        );
    }

    #[test]
    fn a_page_listening_hears_each_reload() {
        let (_work, site) = site(false);
        let server = Server::start(site, 0).unwrap();
        let (status, lines) = events(server.port(), EVENTS).unwrap();
        assert_eq!(status, 200);
        for _ in 0..2 {
            server.reload();
            let heard: Vec<String> = (0..3)
                .map(|_| lines.recv_timeout(Duration::from_secs(30)).unwrap())
                .collect();
            assert_eq!(heard, ["event: reload", "data: reload", ""]);
        }
        let head = request(server.port(), "HEAD", EVENTS).unwrap();
        assert_eq!(head.header("content-type"), Some("text/event-stream"));
    }

    #[test]
    fn a_taken_port_is_an_error() {
        let taken = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let port = taken.local_addr().unwrap().port();
        let (_work, site) = site(false);
        let error = Server::start(site, port).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::AddrInUse);
    }
}
