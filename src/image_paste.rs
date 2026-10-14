//! Local clipboard reads and uploads never use the interactive terminal stream.
use crate::i18n::{Context, Diagnostic, Message, Text};
use anyhow::{Result, bail, ensure};
use std::{
    io::{Read, Seek, Write},
    os::{
        fd::AsRawFd,
        unix::{fs::PermissionsExt, process::CommandExt},
    },
    path::{Path, PathBuf},
    process::{Child, Command, ExitStatus, Stdio},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread::JoinHandle,
    time::{Duration, Instant},
};

const MAX_IMAGE: usize = 20 * 1024 * 1024;
const READ_TIMEOUT: Duration = Duration::from_secs(3);

/// Owns a short, private socket path. Never reuse another session's master.
pub struct Control(tempfile::TempDir);
impl Control {
    pub fn new() -> Result<Self> {
        Ok(Self(
            tempfile::Builder::new()
                .prefix("ssh-tui-")
                .permissions(std::fs::Permissions::from_mode(0o700))
                .tempdir_in("/tmp")?,
        ))
    }
    pub fn path(&self) -> PathBuf {
        self.0.path().join("ssh")
    }
}

#[derive(Debug)]
pub enum Outcome {
    NoImage,
    Uploaded(String),
}

pub struct Job {
    cancel: Arc<AtomicBool>,
    result: mpsc::Receiver<std::result::Result<Outcome, Diagnostic>>,
    worker: Option<JoinHandle<()>>,
}
impl Job {
    pub fn start(socket: PathBuf) -> Self {
        let cancel = Arc::new(AtomicBool::new(false));
        let flag = cancel.clone();
        let (tx, result) = mpsc::channel();
        let worker = std::thread::spawn(move || {
            let result = (|| {
                let Some((bytes, extension)) = clipboard(&flag)? else {
                    return Ok(Outcome::NoImage);
                };
                upload(&socket, &bytes, extension, &flag).map(Outcome::Uploaded)
            })()
            .map_err(|e| Diagnostic::capture(&e));
            let _ = tx.send(result);
        });
        Self {
            cancel,
            result,
            worker: Some(worker),
        }
    }
    pub fn poll(&self) -> Option<std::result::Result<Outcome, Diagnostic>> {
        match self.result.try_recv() {
            Ok(value) => Some(value),
            Err(mpsc::TryRecvError::Empty) => None,
            Err(mpsc::TryRecvError::Disconnected) => {
                Some(Err(Message::plain(Text::ImageWorker).into()))
            }
        }
    }
}
impl Drop for Job {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

#[derive(Default)]
pub struct State {
    pub enabled: bool,
    job: Option<Job>,
    pending: Option<Outcome>,
}
impl State {
    pub fn busy(&self) -> bool {
        self.job.is_some() || self.pending.is_some()
    }
    pub fn cancel(&mut self) {
        self.job.take();
        self.pending.take();
    }
    pub fn reset(&mut self) {
        self.cancel();
        self.enabled = false;
    }
    pub fn start(&mut self, socket: PathBuf) {
        self.job = Some(Job::start(socket));
    }
    /// A result stays with its owning session until that terminal is visible.
    pub fn poll(
        &mut self,
        visible: bool,
    ) -> (bool, Option<std::result::Result<Outcome, Diagnostic>>) {
        let mut changed = false;
        if let Some(result) = self.job.as_ref().and_then(Job::poll) {
            self.job.take();
            changed = true;
            match result {
                Ok(outcome) => self.pending = Some(outcome),
                Err(error) => return (true, Some(Err(error))),
            }
        }
        (
            changed,
            if visible {
                self.pending.take().map(Ok)
            } else {
                None
            },
        )
    }
}

#[derive(Clone, Copy)]
enum Backend {
    X11,
    Wayland,
}
impl Backend {
    fn detect() -> Result<Self> {
        ensure!(cfg!(target_os = "linux"), Message::plain(Text::ImageLinux));
        let session = std::env::var("XDG_SESSION_TYPE").unwrap_or_default();
        if session == "wayland"
            || (session != "x11" && std::env::var_os("WAYLAND_DISPLAY").is_some())
        {
            Ok(Self::Wayland)
        } else if std::env::var_os("DISPLAY").is_some() {
            Ok(Self::X11)
        } else {
            bail!(Message::plain(Text::ImageDisplay))
        }
    }
    fn command(self, mime: Option<&str>) -> Command {
        match self {
            Self::X11 => {
                let mut c = Command::new("xclip");
                c.args([
                    "-selection",
                    "clipboard",
                    "-out",
                    "-target",
                    mime.unwrap_or("TARGETS"),
                ]);
                c
            }
            Self::Wayland => {
                let mut c = Command::new("wl-paste");
                if let Some(mime) = mime {
                    c.args(["--no-newline", "--type", mime]);
                } else {
                    c.arg("--list-types");
                }
                c
            }
        }
    }
}
fn clipboard(cancel: &AtomicBool) -> Result<Option<(Vec<u8>, &'static str)>> {
    let backend = Backend::detect()?;
    let deadline = Instant::now() + READ_TIMEOUT;
    let types = run(backend.command(None), &[], deadline, 65536, cancel)?;
    // Empty clipboards can be reported as a failing selection conversion. Only
    // a successful MIME query establishes that no supported image is present.
    ensure!(
        types.status.success(),
        Message::arg(Text::ImageClipboard, types.error())
    );
    let types = String::from_utf8_lossy(&types.stdout);
    let selected = [("image/png", "png"), ("image/jpeg", "jpg")]
        .into_iter()
        .find(|(mime, _)| types.lines().any(|line| line.trim() == *mime));
    let Some((mime, extension)) = selected else {
        return Ok(None);
    };
    let output = run(
        backend.command(Some(mime)),
        &[],
        deadline,
        MAX_IMAGE,
        cancel,
    )?;
    ensure!(
        output.status.success(),
        Message::arg(Text::ImageClipboard, output.error())
    );
    let valid = if extension == "png" {
        output.stdout.starts_with(b"\x89PNG\r\n\x1a\n")
    } else {
        output.stdout.starts_with(b"\xff\xd8\xff")
    };
    ensure!(valid, Message::plain(Text::ImageInvalid));
    Ok(Some((output.stdout, extension)))
}

// stdin contains only image bytes. The generated command uses fixed syntax and
// numeric length / allowlisted extension, never clipboard text or remote output.
const RECEIVE: &str = r#"set -eu
umask 077
d=$(mktemp -d /tmp/ssh-tui-image.XXXXXXXXXX)
trap 'rm -rf -- "$d"' 0
trap 'exit 1' HUP INT TERM
cat > "$d/partial"
[ "$(wc -c < "$d/partial" | tr -d ' ')" = "$1" ]
mv "$d/partial" "$d/image.$2"
printf 'SSH_TUI_IMAGE:%s/image.%s\n' "$d" "$2"
trap - 0
"#;
fn upload(socket: &Path, bytes: &[u8], extension: &str, cancel: &AtomicBool) -> Result<String> {
    ensure!(
        matches!(extension, "png" | "jpg"),
        Message::plain(Text::ImageInvalid)
    );
    let mut command = Command::new("ssh");
    // Ignore user SSH config for this *slave*: all routing and authentication
    // belong to the existing master. ProxyCommand=false prevents fallback even
    // if the master disappears between socket inspection and connection.
    command
        .args(["-F", "/dev/null", "-S"])
        .arg(socket)
        .args([
            "-T",
            "-o",
            "ControlMaster=no",
            "-o",
            "ControlPersist=no",
            "-o",
            "ProxyCommand=false",
            "-o",
            "BatchMode=yes",
            "-o",
            "ClearAllForwardings=yes",
            "-o",
            "PermitLocalCommand=no",
            "--",
            "ssh-tui-upload",
        ])
        .arg(format!(
            "sh -c '{}' sh {} {}",
            RECEIVE.replace('\'', "'\\''"),
            bytes.len(),
            extension
        ));
    let output = run(
        command,
        bytes,
        Instant::now() + Duration::from_secs(30),
        65536,
        cancel,
    )?;
    ensure!(
        output.status.success(),
        Message::arg(Text::ImageUpload, output.error())
    );
    parse_path(&output.stdout, extension)
}
fn parse_path(output: &[u8], extension: &str) -> Result<String> {
    let text = std::str::from_utf8(output)?;
    let paths: Vec<_> = text
        .lines()
        .filter_map(|l| l.strip_prefix("SSH_TUI_IMAGE:"))
        .collect();
    ensure!(paths.len() == 1, Message::plain(Text::ImageResponse));
    let path = paths[0];
    let body = path
        .strip_prefix("/tmp/ssh-tui-image.")
        .and_then(|p| p.strip_suffix(&format!("/image.{extension}")));
    ensure!(
        body.is_some_and(|s| s.len() == 10 && s.bytes().all(|b| b.is_ascii_alphanumeric())),
        Message::plain(Text::ImageResponse)
    );
    Ok(path.to_string())
}

struct Output {
    status: ExitStatus,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
}
impl Output {
    fn error(&self) -> String {
        String::from_utf8_lossy(&self.stderr).trim().to_owned()
    }
}
struct Process(Child);
impl Drop for Process {
    fn drop(&mut self) {
        // Helpers own a fresh process group; kill descendants holding pipe FDs
        // too, even if the original helper has already exited.
        unsafe {
            libc::kill(-(self.0.id() as i32), libc::SIGKILL);
        }
        let _ = self.0.wait();
    }
}
fn nonblocking(fd: &impl AsRawFd) -> Result<()> {
    let fd = fd.as_raw_fd();
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    ensure!(
        flags >= 0 && unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } >= 0,
        std::io::Error::last_os_error()
    );
    Ok(())
}
fn drain(reader: &mut impl Read, buffer: &mut Vec<u8>, limit: usize) -> Result<()> {
    let mut chunk = [0; 8192];
    // Bound work per tick so an output flood cannot defeat cancellation.
    for _ in 0..32 {
        match reader.read(&mut chunk) {
            Ok(0) => break,
            Ok(n) => {
                ensure!(buffer.len() + n <= limit, Message::plain(Text::ImageLimit));
                buffer.extend_from_slice(&chunk[..n]);
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => break,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e.into()),
        }
    }
    Ok(())
}
fn run(
    mut command: Command,
    input: &[u8],
    deadline: Instant,
    limit: usize,
    cancel: &AtomicBool,
) -> Result<Output> {
    let program = command.get_program().to_string_lossy().to_string();
    let mut file = tempfile::tempfile()?;
    file.write_all(input)?;
    file.rewind()?;
    command
        .stdin(file)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0);
    let mut process = Process(
        command
            .spawn()
            .with_context(|| Message::arg(Text::ImageTool, program))?,
    );
    let mut stdout = process.0.stdout.take().expect("piped stdout");
    let mut stderr = process.0.stderr.take().expect("piped stderr");
    nonblocking(&stdout)?;
    nonblocking(&stderr)?;
    let (mut out, mut err) = (Vec::new(), Vec::new());
    loop {
        ensure!(
            !cancel.load(Ordering::Relaxed),
            Message::plain(Text::ImageCancelled)
        );
        ensure!(
            Instant::now() < deadline,
            Message::plain(Text::ImageTimeout)
        );
        drain(&mut stdout, &mut out, limit)?;
        drain(&mut stderr, &mut err, 65536)?;
        if let Some(status) = process.0.try_wait()? {
            drain(&mut stdout, &mut out, limit)?;
            drain(&mut stderr, &mut err, 65536)?;
            return Ok(Output {
                status,
                stdout: out,
                stderr: err,
            });
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    #[test]
    fn private_control_paths_are_unique() {
        let a = Control::new().unwrap();
        let b = Control::new().unwrap();
        assert_ne!(a.path(), b.path());
        assert_eq!(
            a.0.path().metadata().unwrap().permissions().mode() & 0o777,
            0o700
        );
    }
    #[test]
    fn remote_output_cannot_inject_terminal_input() {
        assert!(
            parse_path(
                b"banner\nSSH_TUI_IMAGE:/tmp/ssh-tui-image.Abc1234567/image.png\n",
                "png"
            )
            .is_ok()
        );
        for path in [
            "/tmp/ssh-tui-image.Abc1234567/../image.png",
            "/tmp/ssh-tui-image.a;evil1234/image.png",
            "/tmp/ssh-tui-image.Abc1234567/image.png\x1b[31m",
        ] {
            assert!(parse_path(format!("SSH_TUI_IMAGE:{path}\n").as_bytes(), "png").is_err());
        }
    }
    #[test]
    fn missing_master_cannot_fall_back_to_network() {
        let dir = Control::new().unwrap();
        assert!(upload(&dir.path(), b"abc", "png", &AtomicBool::new(false)).is_err());
    }
    #[test]
    fn missing_tool_reports_its_name() {
        let error = run(
            Command::new("/nonexistent/ssh-tui-clipboard-test-tool"),
            &[],
            Instant::now() + READ_TIMEOUT,
            100,
            &AtomicBool::new(false),
        )
        .err()
        .expect("missing executable must fail");
        let text = Diagnostic::capture(&error).render(crate::i18n::Language::En);
        assert!(text.contains("ssh-tui-clipboard-test-tool") && text.contains("xclip"));
    }
    #[test]
    fn remote_receiver_rejects_partial_data_and_cleans_up() {
        // Run the exact receiver with a test-private mktemp implementation.
        let dir = tempfile::tempdir().unwrap();
        let destination = dir.path().join("received");
        let script = format!(
            "mktemp() {{ mkdir '{}' && printf '%s\\n' '{}'; }}\n{RECEIVE}",
            destination.display(),
            destination.display(),
        );
        let mut command = Command::new("sh");
        command.args(["-c", &script, "sh", "100", "png"]);
        let result = run(
            command,
            b"short",
            Instant::now() + READ_TIMEOUT,
            1024,
            &AtomicBool::new(false),
        )
        .unwrap();
        assert!(!result.status.success());
        assert!(!destination.exists());
        assert!(result.stdout.is_empty());
    }
    #[test]
    fn subprocess_timeout_limits_and_cancel() {
        let cancel = AtomicBool::new(false);
        let mut command = Command::new("sh");
        command.args(["-c", "sleep 10"]);
        let start = Instant::now();
        assert!(
            run(
                command,
                &[],
                start + Duration::from_millis(50),
                100,
                &cancel
            )
            .is_err()
        );
        assert!(start.elapsed() < Duration::from_secs(2));
        let mut command = Command::new("sh");
        command.args(["-c", "printf 123456"]);
        assert!(run(command, &[], Instant::now() + READ_TIMEOUT, 3, &cancel).is_err());
        cancel.store(true, Ordering::Relaxed);
        assert!(
            run(
                Command::new("cat"),
                &[],
                Instant::now() + READ_TIMEOUT,
                100,
                &cancel
            )
            .is_err()
        );
    }
    #[test]
    fn pending_result_waits_for_own_terminal_and_cancel_discards_it() {
        let mut state = State {
            enabled: true,
            pending: Some(Outcome::Uploaded("/tmp/image.png".into())),
            ..State::default()
        };
        assert!(state.poll(false).1.is_none());
        assert!(state.busy());
        assert!(matches!(state.poll(true).1, Some(Ok(Outcome::Uploaded(_)))));
        state.pending = Some(Outcome::NoImage);
        state.reset();
        assert!(!state.enabled && !state.busy());
        assert!(state.poll(true).1.is_none());
    }
}
