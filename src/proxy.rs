use crate::i18n::{Context, Language, Message, Text};
use anyhow::{Result, bail, ensure};
use base64::{Engine, engine::general_purpose::STANDARD};
use std::io::{Read, Write};
use std::net::{Shutdown, TcpStream, ToSocketAddrs};
use std::time::{Duration, Instant};
use zeroize::Zeroizing;

pub const HEADER_LIMIT: usize = 64 * 1024;
pub const TIMEOUT: Duration = Duration::from_secs(15);

#[derive(Clone, Debug)]
pub struct Endpoint {
    pub host: String,
    pub port: u16,
}

pub fn endpoint(value: &str) -> Result<Endpoint> {
    let url = url::Url::parse(value).context(Message::plain(Text::ProxyUrlInvalid))?;
    ensure!(url.scheme() == "http", Message::plain(Text::HttpOnly));
    ensure!(
        url.username().is_empty() && url.password().is_none(),
        Message::plain(Text::ProxyCredentials)
    );
    ensure!(
        url.path() == "/" && url.query().is_none() && url.fragment().is_none(),
        Message::plain(Text::ProxyUrlParts)
    );
    let host = url
        .host_str()
        .context(Message::plain(Text::ProxyHostMissing))?
        .trim_matches(['[', ']'])
        .to_string();
    let port = url
        .port_or_known_default()
        .context(Message::plain(Text::ProxyPortInvalid))?;
    ensure!(port != 0, Message::plain(Text::ProxyPortZero));
    Ok(Endpoint { host, port })
}

pub fn authority(host: &str, port: u16) -> Result<String> {
    crate::ssh::validate_host(host)?;
    ensure!(port > 0, Message::plain(Text::TargetPortZero));
    let host = host.trim_matches(['[', ']']);
    Ok(if host.contains(':') {
        format!("[{host}]:{port}")
    } else {
        format!("{host}:{port}")
    })
}

pub fn connect(
    endpoint: &Endpoint,
    host: &str,
    port: u16,
    user: &str,
    password: Option<&str>,
    timeout: Duration,
) -> Result<(TcpStream, Vec<u8>)> {
    let dest = authority(host, port)?;
    ensure!(
        !user.contains([':', '\r', '\n', '\0']),
        Message::plain(Text::ProxyUserInvalid)
    );
    let start = Instant::now();
    let address = (endpoint.host.clone(), endpoint.port);
    let (tx, rx) = std::sync::mpsc::sync_channel(1);
    std::thread::spawn(move || {
        let _ = tx.send(address.to_socket_addrs().map(|a| a.collect::<Vec<_>>()));
    });
    let addresses = rx
        .recv_timeout(timeout)
        .context(Message::plain(Text::DnsTimeout))?
        .context(Message::plain(Text::DnsFailed))?;
    let mut stream = None;
    for addr in addresses {
        let remaining = timeout.saturating_sub(start.elapsed());
        if remaining.is_zero() {
            break;
        }
        if let Ok(s) = TcpStream::connect_timeout(&addr, remaining) {
            stream = Some(s);
            break;
        }
    }
    let mut stream = stream.context(Message::plain(Text::ProxyConnect))?;
    stream.set_nodelay(true)?;
    let deadline = Instant::now() + timeout;
    stream.set_write_timeout(Some(timeout))?;
    let mut request = Zeroizing::new(format!("CONNECT {dest} HTTP/1.1\r\nHost: {dest}\r\n"));
    if !user.is_empty() {
        let secret = Zeroizing::new(format!(
            "{user}:{}",
            password.context(Message::plain(Text::ProxyPasswordRequired))?
        ));
        let encoded = Zeroizing::new(STANDARD.encode(secret.as_bytes()));
        request.push_str(&format!(
            "Proxy-Authorization: Basic {}\r\n",
            encoded.as_str()
        ));
    }
    request.push_str("\r\n");
    stream
        .write_all(request.as_bytes())
        .context(Message::plain(Text::SendConnect))?;
    let leftover =
        read_response(&mut stream, deadline).context(Message::plain(Text::ConnectFailed))?;
    stream.set_read_timeout(None)?;
    stream.set_write_timeout(None)?;
    Ok((stream, leftover))
}

fn read_response(stream: &mut TcpStream, deadline: Instant) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    let mut interim = 0;
    loop {
        if let Some(pos) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
            let end = pos + 4;
            ensure!(end <= HEADER_LIMIT, Message::plain(Text::HeaderLimit));
            let mut headers = [httparse::EMPTY_HEADER; 128];
            let mut response = httparse::Response::new(&mut headers);
            ensure!(
                response.parse(&bytes[..end])?.is_complete(),
                Message::plain(Text::ResponseIncomplete)
            );
            let code = response.code.context(Message::plain(Text::StatusMissing))?;
            if (100..200).contains(&code) && code != 101 {
                interim += 1;
                ensure!(interim <= 8, Message::plain(Text::TooManyInterim));
                bytes.drain(..end);
                continue;
            }
            match code {
                200..=299 => return Ok(bytes.split_off(end)),
                407 => bail!(Message::plain(Text::ProxyAuthFailed)),
                403 => bail!(Message::plain(Text::ProxyDenied)),
                _ => bail!(Message::arg(Text::ProxyStatus, code)),
            }
        }
        ensure!(
            bytes.len() < HEADER_LIMIT,
            Message::plain(Text::HeaderLimit)
        );
        let remaining = deadline.saturating_duration_since(Instant::now());
        ensure!(!remaining.is_zero(), Message::plain(Text::HandshakeTimeout));
        stream.set_read_timeout(Some(remaining))?;
        let mut block = [0u8; 4096];
        let n = stream
            .read(&mut block)
            .context(Message::plain(Text::ReadResponse))?;
        ensure!(n > 0, Message::plain(Text::ProxyClosed));
        bytes.extend_from_slice(&block[..n]);
    }
}

pub fn run(url: &str, user: &str, host: &str, port: u16, language: Language) -> Result<()> {
    let endpoint = endpoint(url)?;
    let password = if user.is_empty() {
        None
    } else {
        Some(read_password(language)?)
    };
    let (mut remote, leftover) = connect(
        &endpoint,
        host,
        port,
        user,
        password.as_deref().map(String::as_str),
        TIMEOUT,
    )?;
    drop(password);
    let mut send = remote.try_clone()?;
    std::thread::spawn(move || {
        let _ = std::io::copy(&mut std::io::stdin().lock(), &mut send);
        let _ = send.shutdown(Shutdown::Write);
    });
    let mut out = std::io::stdout().lock();
    out.write_all(&leftover)?;
    out.flush()?;
    // Flush every received chunk: SSH is interactive, stdout must not be line buffered.
    let mut buffer = [0u8; 16384];
    loop {
        let n = remote.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        out.write_all(&buffer[..n])?;
        out.flush()?;
    }
    Ok(())
}

fn read_password(language: Language) -> Result<Zeroizing<String>> {
    use std::os::fd::AsRawFd;
    let mut tty = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open("/dev/tty")
        .context(Message::plain(Text::PasswordTerminal))?;
    let fd = tty.as_raw_fd();
    // The proxy helper inherits the SSH PTY as its controlling terminal.
    let original = unsafe {
        let mut attrs = std::mem::zeroed::<libc::termios>();
        ensure!(
            libc::tcgetattr(fd, &mut attrs) == 0,
            Message::plain(Text::ReadTerminal)
        );
        attrs
    };
    struct Restore(i32, libc::termios);
    impl Drop for Restore {
        fn drop(&mut self) {
            unsafe {
                libc::tcsetattr(self.0, libc::TCSANOW, &self.1);
            }
        }
    }
    let _restore = Restore(fd, original);
    let mut hidden = original;
    hidden.c_lflag &= !libc::ECHO;
    unsafe {
        ensure!(
            libc::tcsetattr(fd, libc::TCSANOW, &hidden) == 0,
            Message::plain(Text::DisableEcho)
        );
    }
    tty.write_all(language.text(Text::PasswordPrompt).as_bytes())?;
    tty.flush()?;
    let mut result = Zeroizing::new(Vec::new());
    let mut byte = [0u8; 1];
    loop {
        ensure!(
            tty.read(&mut byte)? != 0,
            Message::plain(Text::PasswordCancelled)
        );
        if matches!(byte[0], b'\r' | b'\n') {
            break;
        }
        ensure!(result.len() < 4096, Message::plain(Text::PasswordTooLong));
        result.push(byte[0]);
    }
    tty.write_all(b"\r\n")?;
    Ok(Zeroizing::new(std::str::from_utf8(&result)?.to_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;
    fn serve(reply: Vec<u8>, split: bool) -> (Endpoint, std::thread::JoinHandle<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let handle = std::thread::spawn(move || {
            let (mut s, _) = listener.accept().unwrap();
            let mut request = Vec::new();
            let mut b = [0; 1];
            while !request.ends_with(b"\r\n\r\n") {
                s.read_exact(&mut b).unwrap();
                request.push(b[0]);
            }
            if split {
                for chunk in reply.chunks(3) {
                    if s.write_all(chunk).is_err() {
                        break;
                    }
                }
            } else {
                let _ = s.write_all(&reply);
            }
            String::from_utf8(request).unwrap()
        });
        (
            Endpoint {
                host: "127.0.0.1".into(),
                port,
            },
            handle,
        )
    }
    #[test]
    fn basic_auth_fragmented_response_and_coalesced_banner() {
        let (ep, thread) = serve(
            b"HTTP/1.1 200 OK\r\n\r\nSSH-2.0-fixture\r\n".to_vec(),
            false,
        );
        let (mut stream, mut bytes) =
            connect(&ep, "example.org", 22, "user", Some("pass"), TIMEOUT).unwrap();
        stream.read_to_end(&mut bytes).unwrap();
        assert_eq!(bytes, b"SSH-2.0-fixture\r\n");
        let request = thread.join().unwrap();
        assert!(request.contains("CONNECT example.org:22 HTTP/1.1"));
        assert!(request.contains("Proxy-Authorization: Basic dXNlcjpwYXNz"));
        let (ep, thread) = serve(
            b"HTTP/1.1 100 Continue\r\n\r\nHTTP/1.1 204 OK\r\n\r\n".to_vec(),
            true,
        );
        connect(&ep, "::1", 2222, "", None, TIMEOUT).unwrap();
        assert!(thread.join().unwrap().contains("CONNECT [::1]:2222"));
    }
    #[test]
    fn errors_do_not_produce_tunnels() {
        for reply in [
            b"HTTP/1.1 407 Auth\r\n\r\n".to_vec(),
            b"HTTP/1.1 403 Forbidden\r\n\r\n".to_vec(),
            b"garbage\r\n\r\n".to_vec(),
            vec![b'x'; HEADER_LIMIT + 1],
            b"HTTP/1.1 200".to_vec(),
        ] {
            let (ep, thread) = serve(reply, false);
            assert!(connect(&ep, "example.org", 22, "", None, Duration::from_secs(1)).is_err());
            thread.join().unwrap();
        }
    }
    #[test]
    fn endpoint_restrictions() {
        for url in [
            "https://host:443",
            "http://user:secret@host:8080",
            "http://host/a",
            "http://host:0",
        ] {
            assert!(endpoint(url).is_err());
        }
        assert_eq!(endpoint("http://[::1]:7890").unwrap().host, "::1");
        assert!(authority("host\r\nInjected:x", 22).is_err());
    }
    #[test]
    fn stalled_connect_handshake_has_a_deadline() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let endpoint = Endpoint {
            host: "127.0.0.1".into(),
            port: listener.local_addr().unwrap().port(),
        };
        let server = std::thread::spawn(move || {
            let (_stream, _) = listener.accept().unwrap();
            std::thread::sleep(Duration::from_millis(200));
        });
        let start = Instant::now();
        assert!(
            connect(
                &endpoint,
                "example.org",
                22,
                "",
                None,
                Duration::from_millis(30)
            )
            .is_err()
        );
        assert!(start.elapsed() < Duration::from_millis(180));
        server.join().unwrap();
    }
}
