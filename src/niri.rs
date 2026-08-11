use niri_ipc::{Reply, socket::SOCKET_PATH_ENV};
use std::os::unix::fs::{FileTypeExt, MetadataExt};
use tokio::{
    io::{self, AsyncBufRead, AsyncBufReadExt, AsyncWriteExt, BufReader},
    net::UnixStream,
};

const MAX_IPC_LINE_BYTES: usize = 8 * 1024 * 1024;

async fn read_line_limited<R: AsyncBufRead + Unpin>(
    reader: &mut R,
    output: &mut String,
) -> io::Result<usize> {
    output.clear();
    let mut total = 0;
    loop {
        let available = reader.fill_buf().await?;
        if available.is_empty() {
            return Ok(total);
        }
        let chunk_len = available
            .iter()
            .position(|byte| *byte == b'\n')
            .map_or(available.len(), |position| position + 1);
        if total + chunk_len > MAX_IPC_LINE_BYTES {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Niri IPC frame exceeds maximum size",
            ));
        }
        let chunk = &available[..chunk_len];
        let is_line = chunk.ends_with(b"\n");
        let chunk_string = std::str::from_utf8(chunk)
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "Niri IPC frame is not UTF-8"))?
            .to_owned();
        output.push_str(&chunk_string);
        total += chunk_len;
        reader.consume(chunk_len);
        if is_line {
            return Ok(total);
        }
    }
}

pub use niri_ipc::Event;

pub struct Connection(BufReader<UnixStream>);

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
                    return Ok(Self(BufReader::new(s)));
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
        let runtime_dir = if let Some(path) = std::env::var_os("XDG_RUNTIME_DIR") {
            std::path::PathBuf::from(path)
        } else {
            let uid = std::fs::read_to_string("/proc/self/status")
                .ok()
                .and_then(|s| {
                    s.lines()
                        .find(|l| l.starts_with("Uid:"))
                        .and_then(|l| l.split_whitespace().nth(1))
                        .and_then(|u| u.parse::<u32>().ok())
                })
                .ok_or_else(|| {
                    io::Error::new(
                        io::ErrorKind::NotFound,
                        "XDG_RUNTIME_DIR is unset and process UID could not be determined",
                    )
                })?;
            std::path::PathBuf::from(format!("/run/user/{uid}"))
        };
        let runtime_uid = std::fs::metadata(&runtime_dir)
            .ok()
            .map(|metadata| metadata.uid());

        if let Ok(entries) = std::fs::read_dir(&runtime_dir) {
            let mut candidates: Vec<_> = entries
                .flatten()
                .filter_map(|entry| {
                    let path = entry.path();
                    let filename = path.file_name()?.to_str()?;
                    let file_type = entry.file_type().ok()?;
                    let metadata = entry.metadata().ok()?;
                    if !filename.starts_with("niri.wayland-")
                        || !filename.ends_with(".sock")
                        || !file_type.is_socket()
                        || runtime_uid != Some(metadata.uid())
                    {
                        return None;
                    }
                    let modified = metadata.modified().ok()?;
                    Some((path, modified))
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
                        return Ok(Self(BufReader::new(s)));
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
        Ok(Listener(self.0))
    }

    pub async fn push_request(&mut self, req: niri_ipc::Request) -> io::Result<Reply> {
        let mut buf = serde_json::to_string(&req)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        buf.push('\n');
        self.0.get_mut().write_all(buf.as_bytes()).await?;

        buf.clear();
        let _n = read_line_limited(&mut self.0, &mut buf).await?;

        serde_json::from_str(buf.as_str())
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
    }
}

pub struct Listener(BufReader<UnixStream>);

impl Listener {
    pub async fn next_event(&mut self, buf: &mut String) -> io::Result<Option<Event>> {
        let n = read_line_limited(&mut self.0, buf).await?;
        if n == 0 {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "Connection closed",
            ));
        }
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

#[cfg(test)]
mod tests {
    use super::{MAX_IPC_LINE_BYTES, read_line_limited};
    use tokio::io::{AsyncWriteExt, BufReader};

    #[tokio::test]
    async fn bounded_reader_accepts_a_valid_frame() {
        let (mut writer, reader) = tokio::io::duplex(64);
        tokio::spawn(async move {
            writer.write_all(b"{\"ok\":true}\n").await.unwrap();
        });
        let mut reader = BufReader::new(reader);
        let mut output = String::new();
        let size = read_line_limited(&mut reader, &mut output).await.unwrap();
        assert_eq!(size, output.len());
        assert_eq!(output, "{\"ok\":true}\n");
    }

    #[tokio::test]
    async fn bounded_reader_rejects_an_oversized_frame() {
        let (mut writer, reader) = tokio::io::duplex(1024);
        tokio::spawn(async move {
            let frame = vec![b'x'; MAX_IPC_LINE_BYTES + 1];
            writer.write_all(&frame).await.unwrap();
        });
        let mut reader = BufReader::new(reader);
        let mut output = String::new();
        let error = read_line_limited(&mut reader, &mut output)
            .await
            .unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);
    }
}
