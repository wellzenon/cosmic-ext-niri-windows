use niri_ipc::{Reply, socket::SOCKET_PATH_ENV};
use tokio::{
    io::{self, AsyncBufReadExt, AsyncWriteExt, BufReader},
    net::UnixStream,
};

pub use niri_ipc::Event;

pub struct Connection(UnixStream);

impl Connection {
    pub async fn make_connection() -> io::Result<Connection> {
        // 1. Try NIRI_SOCKET env var (set by Niri compositor and propagated into Flatpak)
        if let Some(path) = std::env::var_os(SOCKET_PATH_ENV) {
            let path_buf = std::path::PathBuf::from(&path);
            match UnixStream::connect(&path_buf).await {
                Ok(s) => {
                    log::debug!(
                        "Connected to Niri IPC via NIRI_SOCKET={}",
                        path_buf.display()
                    );
                    return Ok(Self(s));
                }
                Err(e) => {
                    log::warn!(
                        "NIRI_SOCKET={} exists but connection failed: {}",
                        path_buf.display(),
                        e
                    );
                }
            }
        }

        // 2. Scan XDG_RUNTIME_DIR for niri.wayland-*.sock
        let runtime_dir = std::env::var_os("XDG_RUNTIME_DIR")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| {
                // Fallback: determine UID from /proc/self/status or default to 1000
                let uid = std::fs::read_to_string("/proc/self/status")
                    .ok()
                    .and_then(|s| {
                        s.lines()
                            .find(|l| l.starts_with("Uid:"))
                            .and_then(|l| l.split_whitespace().nth(1))
                            .and_then(|u| u.parse::<u32>().ok())
                    })
                    .unwrap_or(1000);
                std::path::PathBuf::from(format!("/run/user/{}", uid))
            });

        if let Ok(entries) = std::fs::read_dir(&runtime_dir) {
            let mut candidates: Vec<_> = entries
                .flatten()
                .filter_map(|entry| {
                    let path = entry.path();
                    let filename = path.file_name()?.to_str()?;
                    if filename.starts_with("niri.wayland-") && filename.ends_with(".sock") {
                        let modified = entry.metadata().ok()?.modified().ok()?;
                        Some((path, modified))
                    } else {
                        None
                    }
                })
                .collect();

            // Sort by most recently modified first
            candidates.sort_by_key(|b| std::cmp::Reverse(b.1));

            for (path, _) in &candidates {
                match UnixStream::connect(path).await {
                    Ok(s) => {
                        log::debug!(
                            "Connected to Niri IPC via scanned socket {}",
                            path.display()
                        );
                        return Ok(Self(s));
                    }
                    Err(e) => {
                        log::warn!(
                            "Found socket {} but connection failed: {}",
                            path.display(),
                            e
                        );
                    }
                }
            }
        }

        Err(io::Error::new(io::ErrorKind::NotFound, {
            let msg = format!(
                "Could not connect to any Niri IPC socket. NIRI_SOCKET={:?}, XDG_RUNTIME_DIR={}",
                std::env::var_os(SOCKET_PATH_ENV),
                runtime_dir.display()
            );
            log::error!("{}", msg);
            msg
        }))
    }

    pub async fn into_listener(mut self) -> io::Result<Listener> {
        let _ = self.push_request(niri_ipc::Request::EventStream).await?;
        let reader = BufReader::new(self.0);
        Ok(Listener(reader))
    }

    pub async fn push_request(&mut self, req: niri_ipc::Request) -> io::Result<Reply> {
        let mut buf = serde_json::to_string(&req)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        buf.push('\n');
        self.0.write_all(buf.as_bytes()).await?;

        buf.clear();
        let mut reader = BufReader::new(&mut self.0);
        reader.read_line(&mut buf).await?;

        serde_json::from_str(buf.as_str())
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
    }
}

pub struct Listener(BufReader<UnixStream>);

impl Listener {
    pub async fn next_event(&mut self, buf: &mut String) -> io::Result<Option<Event>> {
        self.0.read_line(buf).await?;
        match serde_json::from_str(buf) {
            Ok(e) => {
                log::trace!("Niri IPC event received: {:?}", e);
                Ok(Some(e))
            }
            Err(_) => {
                log::debug!("Ignoring unknown Niri IPC event: {}", buf.trim());
                Ok(None)
            }
        }
    }
}
