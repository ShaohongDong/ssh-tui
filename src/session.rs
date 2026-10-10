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
    master: Option<Box<dyn MasterPty + Send>>,
    input: Option<Sender<Vec<u8>>>,
    writer: Option<std::thread::JoinHandle<()>>,
    child: Option<Box<dyn Child + Send + Sync>>,
    receiver: Receiver<Output>,
    reader: Option<std::thread::JoinHandle<()>>,
    end: bool,
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
        let command = crate::ssh::command_with_language(&profile, proxy.as_ref(), language)?;
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
        Ok(Self {
            profile,
            parser: emulator(rows, cols, history),
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
            self.status = Message::arg(Text::Disconnected, status).into();
            self.exited = true;
            self.input.take();
            self.master.take();
            self.child.take();
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
        self.exited = true;
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
