use niri_ipc::{socket::SOCKET_PATH_ENV, Reply};
use tokio::{
    io::{self, AsyncBufReadExt, AsyncWriteExt, BufReader},
    net::UnixStream,
};

pub use niri_ipc::Event;

pub struct Connection(UnixStream);

impl Connection {
    pub async fn make_connection() -> io::Result<Connection> {
        let socket_path = if let Some(path) = std::env::var_os(SOCKET_PATH_ENV) {
            std::path::PathBuf::from(path)
        } else {
            // Fallback: Scan XDG_RUNTIME_DIR for most recently modified niri.wayland-*.sock socket
            let runtime_dir = std::env::var_os("XDG_RUNTIME_DIR").ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::NotFound,
                    "Neither NIRI_SOCKET nor XDG_RUNTIME_DIR are set",
                )
            })?;

            std::fs::read_dir(runtime_dir)?
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
                .max_by_key(|(_, modified)| *modified)
                .map(|(path, _)| path)
                .ok_or_else(|| {
                    io::Error::new(
                        io::ErrorKind::NotFound,
                        "Could not find any active niri.wayland-*.sock socket in XDG_RUNTIME_DIR",
                    )
                })?
        };
        let s = UnixStream::connect(socket_path).await?;
        Ok(Self(s))
    }

    pub async fn to_listener(mut self) -> io::Result<Listener> {
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
            Ok(e) => Ok(Some(e)),
            Err(err) => {
                eprintln!("COSMIC Niri: Failed to deserialize event: {err}. Raw: {buf}");
                Ok(None)
            }
        }
    }
}
