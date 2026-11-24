//! The profile's `wake.sock`: one JSON line per wake.

use std::path::Path;

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::mpsc::{UnboundedReceiver, unbounded_channel};

use super::Wake;

/// MCP side: tells a listening launcher, if any.
pub async fn notify(socket: &Path, wake: &Wake) {
    if !socket.exists() {
        return;
    }
    let Ok(mut stream) = UnixStream::connect(socket).await else { return };
    let line = format!("{}\n", serde_json::to_string(wake).expect("wakes always serialize"));
    let _ = stream.write_all(line.as_bytes()).await;
}

/// Launcher side: listens on the socket and forwards every wake.
pub fn listen(socket: &Path) -> std::io::Result<UnboundedReceiver<Wake>> {
    let _ = std::fs::remove_file(socket);
    let listener = UnixListener::bind(socket)?;
    let (sender, receiver) = unbounded_channel();
    tokio::spawn(async move {
        while let Ok((stream, _)) = listener.accept().await {
            let sender = sender.clone();
            tokio::spawn(async move {
                let mut lines = BufReader::new(stream).lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    if let Ok(wake) = serde_json::from_str::<Wake>(&line) {
                        let _ = sender.send(wake);
                    }
                }
            });
        }
    });
    Ok(receiver)
}
