use crate::i18n::{Context, Diagnostic, Language, Message, Text};
use crate::{
    config::{Profile, Proxy},
    terminal::{Emulator, emulator},
};
use anyhow::{Result, ensure};
use portable_pty::{Child, MasterPty, PtySize, native_pty_system};
use std::{
    io::{Read, Write},
    sync::mpsc::{self, Receiver, Sender},
    time::{Duration, Instant},
};

enum Output {
    Data(Vec<u8>),
    End,
    Error(String),
    WriteError(String),
}
pub struct Session {
    pub profile: Profile,
    pub parser: Emulator,
    pub status: Diagnostic,
    pub exited: bool,
    pub route_label: String,
    pub image: crate::image_paste::State,
    pub image_error: bool,
    control: Option<crate::image_paste::Control>,
    master: Option<Box<dyn MasterPty + Send>>,
    input: Option<Sender<Vec<u8>>>,
    writer: Option<std::thread::JoinHandle<()>>,
    child: Option<Box<dyn Child + Send + Sync>>,
    receiver: Receiver<Output>,
    reader: Option<std::thread::JoinHandle<()>>,
    end: bool,
    auth: Option<crate::askpass::Broker>,
    pub unread: bool,
}
impl Session {
    pub fn open(
        profile: Profile,
        proxy: Option<Proxy>,
        rows: u16,
        cols: u16,
        history: usize,
        language: Language,
    ) -> Result<Self> {
        Self::open_with_credentials(profile, proxy, rows, cols, history, language, None)
    }
    pub fn open_with_credentials(
        profile: Profile,
        proxy: Option<Proxy>,
        rows: u16,
        cols: u16,
        history: usize,
        language: Language,
        store: Option<std::sync::Arc<dyn crate::credentials::CredentialStore>>,
    ) -> Result<Self> {
        let control = if cfg!(target_os = "linux") {
            Some(crate::image_paste::Control::new()?)
        } else {
            None
        };
        let socket = control.as_ref().map(|c| c.path());
        let mut command = crate::ssh::command_with_options(
            &profile,
            proxy.as_ref(),
            language,
            None,
            socket.as_deref(),
        )?;
        let mut auth = None;
        let mut warning = None;
        if let Some(id) = &profile.credential_id
            && let Some(store) = store
        {
            let target = crate::ssh::effective_target(&profile)?;
            if target.has_external_route {
                warning = Some(Text::JumpPasswordManual);
            } else if target
                .preferred_authentications
                .split(',')
                .all(|method| method == "keyboard-interactive")
            {
                warning = Some(Text::ManualPasswordFallback);
            } else {
                let prompt = format!("{}@{}'s password: ", target.user, target.password_host);
                match crate::askpass::Broker::new(store, id.clone(), prompt, language) {
                    Ok(broker) => {
                        // Insert before the destination. Keep the configured methods,
                        // but never send a saved password to server-authored challenges.
                        let preferred = target
                            .preferred_authentications
                            .split(',')
                            .filter(|method| *method != "keyboard-interactive")
                            .collect::<Vec<_>>()
                            .join(",");
                        // CommandBuilder appends args, so reconstruct with this option
                        // through the SSH command builder rather than after the host.
                        command = crate::ssh::command_with_options(
                            &profile,
                            proxy.as_ref(),
                            language,
                            Some(&preferred),
                            socket.as_deref(),
                        )?;
                        broker.configure(&mut command, language);
                        auth = Some(broker);
                    }
                    Err(_) => warning = Some(Text::AskpassUnavailable),
                }
            }
        }
        let pair = native_pty_system().openpty(size(rows, cols))?;
        let mut reader = pair.master.try_clone_reader()?;
        let mut writer = pair.master.take_writer()?;
        let child = pair
            .slave
            .spawn_command(command)
            .context(Message::plain(Text::StartSession))?;
        drop(pair.slave);
        let (sender, receiver) = mpsc::sync_channel(64);
        // A burst of key events must not lose bytes while the PTY writer is
        // scheduled. Output stays bounded; input comes only from the local user.
        let (input, incoming) = mpsc::channel::<Vec<u8>>();
        let writer_sender = sender.clone();
        let writer_handle = std::thread::spawn(move || {
            while let Ok(bytes) = incoming.recv() {
                let bytes = zeroize::Zeroizing::new(bytes);
                if let Err(error) = writer.write_all(&bytes).and_then(|_| writer.flush()) {
                    let _ = writer_sender.send(Output::WriteError(error.to_string()));
                    break;
                }
            }
        });
        let handle = std::thread::spawn(move || {
            let mut buffer = [0u8; 8192];
            loop {
                match reader.read(&mut buffer) {
                    Ok(0) => break,
                    Ok(n) => {
                        if sender.send(Output::Data(buffer[..n].to_vec())).is_err() {
                            return;
                        }
                    }
                    Err(e) if e.raw_os_error() == Some(libc::EIO) => break,
                    Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                    Err(e) => {
                        let _ = sender.send(Output::Error(e.to_string()));
                        break;
                    }
                }
            }
            let _ = sender.send(Output::End);
        });
        let mut parser = emulator(rows, cols, history);
        if let Some(text) = warning {
            parser.process(format!("{}\r\n", language.text(text)).as_bytes());
        }
        Ok(Self {
            profile,
            control,
            image: crate::image_paste::State::default(),
            image_error: false,
            parser,
            status: Message::plain(Text::Running).into(),
            exited: false,
            route_label: proxy.map_or("OpenSSH".into(), |p| format!("HTTP: {}", p.name)),
            master: Some(pair.master),
            input: Some(input),
            writer: Some(writer_handle),
            child: Some(child),
            receiver,
            reader: Some(handle),
            end: false,
            auth,
            unread: false,
        })
    }
    pub fn poll(&mut self, active: bool) -> Result<bool> {
        let mut changed = false;
        // Bound work per tick so noisy background sessions cannot starve input.
        for _ in 0..32 {
            match self.receiver.try_recv() {
                Ok(Output::Data(bytes)) => {
                    self.parser.process(&bytes);
                    self.unread |= !active;
                    changed = true;
                }
                Ok(Output::End) => {
                    self.end = true;
                    break;
                }
                Ok(Output::Error(e)) => {
                    self.status = e.into();
                    self.end = true;
                    break;
                }
                Ok(Output::WriteError(e)) => {
                    self.status = Message::arg(Text::InputFailed, e).into();
                    changed = true;
                }
                Err(_) => break,
            }
        }
        let replies = std::mem::take(&mut self.parser.callbacks_mut().pending);
        if !replies.is_empty() && !self.exited {
            self.write(&replies)?;
        }
        if self.end
            && !self.exited
            && let Some(status) = self
                .child
                .as_mut()
                .map(|c| c.try_wait())
                .transpose()?
                .flatten()
        {
            self.image.reset();
            self.control.take();
            self.image_error = false;
            self.status = Message::arg(Text::Disconnected, status).into();
            self.exited = true;
            self.input.take();
            self.master.take();
            self.child.take();
            self.auth.take();
            if let Some(handle) = self.reader.take() {
                let _ = handle.join();
            }
            if let Some(handle) = self.writer.take() {
                let _ = handle.join();
            }
            changed = true;
        }
        if active {
            self.unread = false;
        }
        Ok(changed)
    }
    pub fn toggle_images(&mut self) -> Result<()> {
        ensure!(cfg!(target_os = "linux"), Message::plain(Text::ImageLinux));
        ensure!(!self.exited, Message::plain(Text::SessionDisconnected));
        self.image.cancel();
        self.image_error = false;
        self.image.enabled = !self.image.enabled;
        self.status = Message::plain(if self.image.enabled {
            Text::ImageOn
        } else {
            Text::ImageOff
        })
        .into();
        Ok(())
    }
    pub fn start_image(&mut self) -> Result<()> {
        ensure!(!self.exited, Message::plain(Text::SessionDisconnected));
        let control = self
            .control
            .as_ref()
            .context(Message::plain(Text::ImageLinux))?;
        if !self.image.busy() {
            self.image_error = false;
            self.image.start(control.path());
        }
        Ok(())
    }
    pub fn cancel_image(&mut self) {
        self.image.cancel();
        self.image_error = false;
        self.status = Message::plain(Text::ImageCancelled).into();
    }
    pub fn poll_image(&mut self, visible: bool) -> Result<bool> {
        if self.exited {
            self.image.reset();
            return Ok(false);
        }
        let (changed, outcome) = self.image.poll(visible);
        let delivered = outcome.is_some();
        if let Some(outcome) = outcome {
            match outcome {
                Ok(crate::image_paste::Outcome::NoImage) => self.write(&[22])?,
                Ok(crate::image_paste::Outcome::Uploaded(path)) => {
                    self.write(&crate::input::paste(
                        &path,
                        self.parser.screen().bracketed_paste(),
                    ))?;
                    self.status = Message::arg(Text::ImageReady, path).into();
                }
                Err(error) => {
                    self.image_error = true;
                    self.status = error;
                }
            }
        }
        Ok(changed || delivered)
    }
    pub fn write(&mut self, bytes: &[u8]) -> Result<()> {
        ensure!(!self.exited, Message::plain(Text::SessionDisconnected));
        if !bytes.is_empty() {
            let input = self
                .input
                .as_ref()
                .context(Message::plain(Text::InputClosed))?;
            input
                .send(bytes.to_vec())
                .map_err(|_| anyhow::anyhow!(Message::plain(Text::InputClosed)))?;
        }
        Ok(())
    }
    pub fn resize(&mut self, rows: u16, cols: u16) -> Result<()> {
        self.parser.screen_mut().set_size(rows.max(1), cols.max(1));
        if let Some(master) = &self.master {
            master.resize(size(rows, cols))?;
        }
        Ok(())
    }
    pub fn close(&mut self) {
        self.image.reset();
        if let Some(mut child) = self.child.take() {
            // SSH and its ProxyCommand share this newly created process group.
            if let Some(pid) = child.process_id() {
                unsafe {
                    libc::kill(-(pid as i32), libc::SIGHUP);
                }
            }
            let deadline = Instant::now() + Duration::from_millis(250);
            while Instant::now() < deadline {
                if child.try_wait().ok().flatten().is_some() {
                    break;
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            if child.try_wait().ok().flatten().is_none() {
                if let Some(pid) = child.process_id() {
                    unsafe {
                        libc::kill(-(pid as i32), libc::SIGKILL);
                    }
                }
                let _ = child.kill();
            }
            let _ = child.wait();
        }
        self.input.take();
        self.master.take();
        self.auth.take();
        self.exited = true;
        self.control.take();
        // Dropping the receiver unblocks a producer waiting on a full queue.
        let (_, empty) = mpsc::channel();
        self.receiver = empty;
        if let Some(handle) = self.reader.take() {
            let _ = handle.join();
        }
        if let Some(handle) = self.writer.take() {
            let _ = handle.join();
        }
    }
}
impl Drop for Session {
    fn drop(&mut self) {
        self.close();
    }
}
pub fn size(rows: u16, cols: u16) -> PtySize {
    PtySize {
        rows: rows.max(1),
        cols: cols.max(1),
        pixel_width: 0,
        pixel_height: 0,
    }
}
