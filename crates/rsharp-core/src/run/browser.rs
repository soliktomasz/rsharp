//! The browser a web project opens: the address a profile's URL names, and how a launch knows the program is up
//! before the page opens. Nothing a program writes is read, so a program that says nothing, or says it in another
//! language, opens its browser the same.

use std::net::{TcpStream, ToSocketAddrs};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

/// A URL a browser can open: its scheme, the host a browser is given and its port, and the text it was written as.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Address {
    pub scheme: String,
    /// The host a browser opens: a URL bound to every interface (`0.0.0.0`, `[::]`, `*`, `+`) is this machine's.
    pub host: String,
    pub port: u16,
    /// The path and query as written, the slash it starts with included; empty for none.
    pub path: String,
    /// The URL as the profile wrote it, which is what the browser is given unless its host is every interface.
    text: String,
    wildcard: bool,
}

/// How often a launch tries the address, and how long each try waits: Passenger's timing, whose first try is 300 ms
/// after the start, then one every half second for ten tries, every second for ten more, and every five seconds for
/// eighty more (about seven minutes).
#[derive(Clone, Debug)]
pub struct Tries {
    pub first: Duration,
    pub attempt: Duration,
    /// The pauses between tries.
    pub pauses: Vec<Duration>,
}

impl Default for Tries {
    fn default() -> Self {
        let mut pauses = vec![Duration::from_millis(500); 10];
        pauses.extend(vec![Duration::from_secs(1); 10]);
        pauses.extend(vec![Duration::from_secs(5); 80]);
        Self { first: Duration::from_millis(300), attempt: Duration::from_secs(1), pauses }
    }
}

impl Tries {
    /// Three tries a moment apart, for a test that must not wait minutes for a program that never listens.
    pub fn quick() -> Self {
        Self { first: Duration::ZERO, attempt: Duration::from_millis(200), pauses: vec![Duration::from_millis(50); 3] }
    }
}

impl Address {
    /// A launch's browser URL as an address a browser opens (http or https); none for another kind.
    pub fn parse(url: &str) -> Option<Address> {
        let text = url.trim();
        let (scheme, rest) = text.split_once("://")?;
        let scheme = scheme.to_ascii_lowercase();
        let default_port = match scheme.as_str() {
            "http" => 80,
            "https" => 443,
            _ => return None,
        };
        let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
        let written = match authority.rsplit_once(':') {
            Some((host, port)) => (host, Some(port.parse::<u16>().ok()?)),
            None => (authority, None),
        };
        let bare = written.0.trim_matches(['[', ']']);
        if bare.is_empty() {
            return None;
        }
        let wildcard = matches!(bare, "0.0.0.0" | "::" | "*" | "+");
        let port = written.1.unwrap_or(default_port);
        let path = rest.get(authority.len()..).unwrap_or_default().to_string();
        Some(Address {
            scheme,
            host: if wildcard { "localhost".into() } else { bare.to_string() },
            port,
            path,
            text: text.to_string(),
            wildcard,
        })
    }

    /// The address as the browser is given it: one bound to every interface is this machine's, which a browser
    /// cannot open as written.
    pub fn shown(&self) -> String {
        if !self.wildcard {
            return self.text.clone();
        }
        let port = if (self.scheme == "http" && self.port == 80) || (self.scheme == "https" && self.port == 443) {
            String::new()
        } else {
            format!(":{}", self.port)
        };
        format!("{}://localhost{port}{}", self.scheme, self.path)
    }

    /// Rider's words when the program never listened at the address.
    pub fn not_responding(&self) -> String {
        format!("Unable to open {} in the browser because the URL does not respond", self.shown())
    }
}

/// Opens `url` in the person's browser as the desktop opens one: `open` on macOS, `xdg-open` on Linux, and Windows'
/// `rundll32 url.dll,FileProtocolHandler`, which hands the URL to the shell's own handler. No shell parses it, so a
/// query's `&`, `%` and `^` reach the browser as the profile wrote them. The browser is the one program that outlives
/// the IDE, so it is started detached, and an opener that fails says why.
pub fn open(url: &str) -> Result<(), String> {
    #[cfg(windows)]
    let (program, arguments): (&str, Vec<&str>) = ("rundll32", vec!["url.dll,FileProtocolHandler", url]);
    #[cfg(not(windows))]
    let (program, arguments): (&str, Vec<&str>) =
        if cfg!(target_os = "macos") { ("open", vec![url]) } else { ("xdg-open", vec![url]) };
    let found = opener(program)?;
    let mut command = std::process::Command::new(found);
    command.args(arguments);
    crate::process::spawn_detached(command)
        .map(|_| ())
        .map_err(|error| format!("{program} could not be started: {error}"))
}

/// Where the desktop's opener is: on the PATH, as the desktop puts it there.
#[cfg(not(windows))]
fn opener(program: &str) -> Result<std::path::PathBuf, String> {
    crate::programs::find(program, &[]).ok_or_else(|| format!("{program} is not installed"))
}

/// Where Windows' opener is: System32 holds it, which a GUI program's PATH need not name.
#[cfg(windows)]
fn opener(program: &str) -> Result<std::path::PathBuf, String> {
    let system = std::env::var_os("SystemRoot").map(std::path::PathBuf::from).unwrap_or_else(|| r"C:\Windows".into());
    let found = system.join("System32").join(format!("{program}.exe"));
    if found.is_file() {
        return Ok(found);
    }
    crate::programs::find(program, &[]).ok_or_else(|| format!("{program} is not installed"))
}

/// Waits for something to listen at the address's host and port: true once it does, false when the tries run out,
/// none when the wait was ended first (the program ended, or Stop).
pub fn listening(address: &Address, tries: &Tries, ended: &AtomicBool) -> Option<bool> {
    if wait(tries.first, ended) {
        return None;
    }
    for (tried, pause) in tries.pauses.iter().enumerate() {
        if let Some(connection) = connect(address, tries.attempt) {
            drop(connection);
            return Some(true);
        }
        if tried + 1 == tries.pauses.len() {
            break;
        }
        if wait(*pause, ended) {
            return None;
        }
    }
    Some(false)
}

fn connect(address: &Address, attempt: Duration) -> Option<TcpStream> {
    let host = if address.host.contains(':') { format!("[{}]", address.host) } else { address.host.clone() };
    let found = (host.as_str(), address.port).to_socket_addrs().ok()?.next()?;
    TcpStream::connect_timeout(&found, attempt).ok()
}

/// Waits as long as asked, in slices, saying whether the launch ended meanwhile.
fn wait(duration: Duration, ended: &AtomicBool) -> bool {
    let mut left = duration;
    while !left.is_zero() {
        if ended.load(Ordering::Acquire) {
            return true;
        }
        let slice = left.min(Duration::from_millis(25));
        std::thread::sleep(slice);
        left -= slice;
    }
    ended.load(Ordering::Acquire)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_profiles_url_is_read_as_an_address_a_browser_opens() {
        // The URL a launch opens is the profile's, and only http and https are URLs a browser is given.
        let plain = Address::parse("http://localhost:5000").unwrap();
        assert_eq!(
            (plain.scheme.as_str(), plain.host.as_str(), plain.port, plain.path.as_str()),
            ("http", "localhost", 5000, "")
        );
        assert_eq!(plain.shown(), "http://localhost:5000");
        assert_eq!(Address::parse("https://localhost").unwrap().port, 443);
        let path = Address::parse("http://example.test:8080/swagger?x=1").unwrap();
        assert_eq!(
            (path.path.as_str(), path.shown().as_str()),
            ("/swagger?x=1", "http://example.test:8080/swagger?x=1")
        );
        assert_eq!(Address::parse("ftp://example.test").map(|a| a.shown()), None, "only http and https are opened");
        assert_eq!(Address::parse("not a url").map(|a| a.shown()), None);
        assert_eq!(Address::parse("http://*:5000").unwrap().shown(), "http://localhost:5000");
        let wildcard = Address::parse("http://0.0.0.0:5000/api").unwrap();
        assert_eq!(wildcard.shown(), "http://localhost:5000/api", "a browser cannot open 0.0.0.0 as written");
        assert_eq!(wildcard.host, "localhost");
        assert_eq!(Address::parse("https://[::]:7001").unwrap().shown(), "https://localhost:7001");
        assert_eq!(Address::parse("http://+:8080").map(|a| a.shown()), Some("http://localhost:8080".into()));
        assert!(plain.not_responding().contains("http://localhost:5000"));
    }

    #[test]
    fn a_listener_that_answers_is_waited_for_and_one_that_never_listens_gives_up() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let address = Address::parse(&format!("http://127.0.0.1:{port}")).unwrap();
        let ended = AtomicBool::new(false);
        assert_eq!(listening(&address, &Tries::quick(), &ended), Some(true), "a listening program opens its browser");

        let closed = Address::parse("http://127.0.0.1:1").unwrap();
        assert_eq!(listening(&closed, &Tries::quick(), &ended), Some(false), "a program that never listens gives up");

        let stopped = AtomicBool::new(true);
        assert_eq!(listening(&address, &Tries::quick(), &stopped), None, "a run that ended stops the wait");
    }
}
