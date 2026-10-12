//! A session-scoped password broker. The helper's stdout is OpenSSH's private pipe.
use crate::{
    credentials::{CredentialStore, new_id, validate_password},
    i18n::{Context, Language, Message, Text},
};
use anyhow::{Result, ensure};
use portable_pty::CommandBuilder;
use std::{
    fs::{self, OpenOptions},
    io::{Read, Write},
    net::Shutdown,
    os::{
        fd::AsRawFd,
        unix::{
            fs::PermissionsExt,
            net::{UnixListener, UnixStream},
        },
    },
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread::{self, JoinHandle},
    time::Duration,
};
use zeroize::Zeroizing;

const LIMIT: u64 = 16_384;
const SOCKET_ENV: &str = "SSH_TUI_ASKPASS_SOCKET";
const TOKEN_ENV: &str = "SSH_TUI_ASKPASS_TOKEN";

pub struct Broker {
    dir: tempfile::TempDir,
    token: String,
    stopped: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl Broker {
    pub fn new(
        store: Arc<dyn CredentialStore>,
        id: String,
        prompt: String,
        language: Language,
    ) -> Result<Self> {
        let dir = tempfile::Builder::new().prefix("ssh-tui-auth-").tempdir()?;
        fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o700))?;
        let socket = dir.path().join("socket");
        let listener = UnixListener::bind(&socket)?;
        fs::set_permissions(&socket, fs::Permissions::from_mode(0o600))?;
        listener.set_nonblocking(true)?;
        // A wrapper lets OpenSSH supply its prompt as the final argument. It
        // contains only an executable path, never credentials or the token.
        let exe = std::env::current_exe()?;
        let exe = exe.to_str().context(Message::plain(Text::ExeUtf8))?;
        let quoted = format!("'{}'", exe.replace('\'', "'\\''"));
        let helper = dir.path().join("askpass");
        fs::write(
            &helper,
            format!("#!/bin/sh\nexec {quoted} __ssh-askpass \"$@\"\n"),
        )?;
        fs::set_permissions(&helper, fs::Permissions::from_mode(0o700))?;
        let token = new_id();
        let expected_token = token.clone();
        let stopped = Arc::new(AtomicBool::new(false));
        let stop = stopped.clone();
        let thread = thread::spawn(move || {
            let mut attempted = false;
            let mut supplied = false;
            while !stop.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
                        let _ = stream.set_write_timeout(Some(Duration::from_secs(2)));
                        let mut request = String::new();
                        if (&mut stream)
                            .take(LIMIT + 1)
                            .read_to_string(&mut request)
                            .is_err()
                            || request.len() as u64 > LIMIT
                        {
                            continue;
                        }
                        let mut parts = request.splitn(3, '\n');
                        if parts.next() != Some(expected_token.as_str()) {
                            continue;
                        }
                        let hint = parts.next().unwrap_or_default();
                        let actual = parts.next().unwrap_or_default();
                        let response = if hint == "hidden" && actual == prompt {
                            if attempted && supplied {
                                let _ = stream.write_all(&[2]);
                                language
                                    .text(Text::SavedPasswordRejected)
                                    .as_bytes()
                                    .to_vec()
                            } else if attempted {
                                let _ = stream.write_all(&[0]);
                                Vec::new()
                            } else {
                                attempted = true;
                                match store.get(&id).and_then(|password| {
                                    validate_password(&password)?;
                                    Ok(password)
                                }) {
                                    Ok(password) => {
                                        supplied = true;
                                        if stream.write_all(&[1]).is_ok() {
                                            let _ = stream.write_all(password.as_bytes());
                                        }
                                        continue;
                                    }
                                    Err(error) => {
                                        let _ = stream.write_all(&[2]);
                                        crate::i18n::Diagnostic::capture(&error)
                                            .render(language)
                                            .into_bytes()
                                    }
                                }
                            }
                        } else {
                            let _ = stream.write_all(&[0]);
                            Vec::new()
                        };
                        let _ = stream.write_all(&response);
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(20));
                    }
                    Err(_) => break,
                }
            }
        });
        Ok(Self {
            dir,
            token,
            stopped,
            thread: Some(thread),
        })
    }

    pub fn configure(&self, command: &mut CommandBuilder, language: Language) {
        command.env("SSH_ASKPASS", self.dir.path().join("askpass"));
        command.env("SSH_ASKPASS_REQUIRE", "force");
        command.env(SOCKET_ENV, self.dir.path().join("socket"));
        command.env(TOKEN_ENV, &self.token);
        command.env("SSH_TUI_ASKPASS_LANGUAGE", language.code());
        command.env_remove("SSH_ASKPASS_PROMPT");
    }

    #[cfg(test)]
    fn socket(&self) -> PathBuf {
        self.dir.path().join("socket")
    }
}

impl Drop for Broker {
    fn drop(&mut self) {
        self.stopped.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn request(socket: PathBuf, token: &str, hint: &str, prompt: &str) -> Result<Zeroizing<Vec<u8>>> {
    let mut stream = UnixStream::connect(socket)?;
    stream.set_read_timeout(Some(Duration::from_secs(30)))?;
    stream.set_write_timeout(Some(Duration::from_secs(2)))?;
    write!(stream, "{token}\n{hint}\n{prompt}")?;
    stream.shutdown(Shutdown::Write)?;
    let mut response = Zeroizing::new(Vec::new());
    stream.take(LIMIT + 1).read_to_end(&mut response)?;
    ensure!(
        response.len() as u64 <= LIMIT && !response.is_empty(),
        Message::plain(Text::AskpassUnavailable)
    );
    Ok(response)
}

/// Called only by the session's askpass executable wrapper, before normal CLI parsing.
pub fn run(prompt: &str) -> Result<()> {
    let language = if std::env::var("SSH_TUI_ASKPASS_LANGUAGE").as_deref() == Ok("zh") {
        Language::Zh
    } else {
        Language::En
    };
    let hint = std::env::var("SSH_ASKPASS_PROMPT").unwrap_or_else(|_| "hidden".into());
    let response = std::env::var_os(SOCKET_ENV)
        .zip(std::env::var(TOKEN_ENV).ok())
        .map(|(socket, token)| request(socket.into(), &token, &hint, prompt));
    let mut tty = OpenOptions::new()
        .read(true)
        .write(true)
        .open("/dev/tty")
        .context(Message::plain(Text::AskpassCancelled))?;
    if let Some(Ok(response)) = response {
        if response[0] == 1 {
            std::io::stdout().write_all(&response[1..])?;
            return Ok(());
        }
        if response[0] == 2 {
            tty.write_all(&response[1..])?;
            tty.write_all(b"\r\n")?;
        }
    } else {
        writeln!(tty, "{}", language.text(Text::AskpassUnavailable))?;
    }
    if hint == "none" {
        writeln!(tty, "{prompt}")?;
        return Ok(());
    }
    let echo = manual_echo(&hint, prompt);
    let response = read_tty(&mut tty, prompt, echo)?;
    if hint == "confirm" && response.is_empty() {
        std::io::stdout().write_all(b"no")?;
    } else {
        std::io::stdout().write_all(response.as_bytes())?;
    }
    Ok(())
}

fn manual_echo(hint: &str, prompt: &str) -> bool {
    hint == "confirm"
        || (prompt.contains("Are you sure you want to continue connecting")
            && ["(yes/no/[fingerprint])?", "(yes/no)?"]
                .iter()
                .any(|suffix| prompt.trim_end().ends_with(suffix)))
}

fn read_tty(tty: &mut fs::File, prompt: &str, echo: bool) -> Result<Zeroizing<String>> {
    let fd = tty.as_raw_fd();
    let mut original = unsafe { std::mem::zeroed::<libc::termios>() };
    ensure!(
        unsafe { libc::tcgetattr(fd, &mut original) } == 0,
        Message::plain(Text::ReadTerminal)
    );
    struct Restore(i32, libc::termios);
    impl Drop for Restore {
        fn drop(&mut self) {
            unsafe {
                libc::tcsetattr(self.0, libc::TCSANOW, &self.1);
            }
        }
    }
    let _restore = Restore(fd, original);
    let mut attrs = original;
    if echo {
        attrs.c_lflag |= libc::ECHO;
    } else {
        attrs.c_lflag &= !libc::ECHO;
    }
    ensure!(
        unsafe { libc::tcsetattr(fd, libc::TCSANOW, &attrs) } == 0,
        Message::plain(Text::DisableEcho)
    );
    tty.write_all(prompt.as_bytes())?;
    tty.flush()?;
    let mut bytes = Zeroizing::new(Vec::new());
    let mut byte = Zeroizing::new([0u8; 1]);
    loop {
        ensure!(
            tty.read(&mut *byte)? != 0,
            Message::plain(Text::AskpassCancelled)
        );
        if matches!(byte[0], b'\r' | b'\n') {
            break;
        }
        ensure!(
            bytes.len() < 1023 && byte[0] != 0,
            Message::plain(Text::SshPasswordInvalid)
        );
        bytes.push(byte[0]);
    }
    if !echo {
        tty.write_all(b"\r\n")?;
    }
    Ok(Zeroizing::new(std::str::from_utf8(&bytes)?.to_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::credentials::{CredentialStore, tests::MemoryStore};

    #[test]
    fn private_key_paths_cannot_enable_password_echo() {
        assert!(!manual_echo(
            "hidden",
            "Enter passphrase for key '/tmp/(yes/no-key)': "
        ));
        assert!(manual_echo(
            "hidden",
            "Are you sure you want to continue connecting (yes/no/[fingerprint])? "
        ));
        assert!(manual_echo("confirm", "Allow access?"));
    }

    #[test]
    fn missing_password_is_not_later_reported_as_rejected() {
        let broker = Broker::new(
            Arc::new(MemoryStore::default()),
            new_id(),
            "alice@server's password: ".into(),
            Language::En,
        )
        .unwrap();
        assert_eq!(
            request(
                broker.socket(),
                &broker.token,
                "hidden",
                "alice@server's password: "
            )
            .unwrap()[0],
            2
        );
        assert_eq!(
            &*request(
                broker.socket(),
                &broker.token,
                "hidden",
                "alice@server's password: "
            )
            .unwrap(),
            &[0]
        );
    }

    #[test]
    fn broker_is_single_use_target_bound_and_does_not_answer_other_prompts() {
        let store = Arc::new(MemoryStore::default());
        let id = new_id();
        store.set(&id, " test-secret ").unwrap();
        let prompt = "alice@server's password: ";
        let broker = Broker::new(store, id, prompt.into(), Language::En).unwrap();
        for (hint, value) in [
            ("hidden", "Password:"),
            ("confirm", prompt),
            ("hidden", "alice@jump's password: "),
        ] {
            assert_eq!(
                request(broker.socket(), &broker.token, hint, value)
                    .unwrap()
                    .as_slice(),
                &[0]
            );
        }
        assert!(request(broker.socket(), "wrong-token", "hidden", prompt).is_err());
        let response = request(broker.socket(), &broker.token, "hidden", prompt).unwrap();
        assert_eq!(&response[..], b"\x01 test-secret ");
        assert_eq!(
            request(broker.socket(), &broker.token, "hidden", prompt).unwrap()[0],
            2
        );
        let path = broker.dir.path().to_owned();
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o700
        );
        assert_eq!(
            fs::metadata(broker.socket()).unwrap().permissions().mode() & 0o777,
            0o600
        );
        drop(broker);
        assert!(!path.exists());
    }
}
