//! Manual test client for vmuxd_proto — see docs/session-reattach-design.md.
//!
//! Connects, prints the replay + live output stream, forwards stdin lines
//! as input. Exit with Ctrl+C or an empty line followed by "exit". Run it,
//! type a command, quit it (Ctrl+C), run it again — the daemon's session
//! (and whatever the shell was doing) should still be there, proving the
//! session survived independent of this client's lifetime.

use serde::{Deserialize, Serialize};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::windows::named_pipe::ClientOptions;

const PIPE_NAME: &str = r"\\.\pipe\vmux-proto";

#[derive(Debug, Serialize, Deserialize)]
enum Request {
    Write(Vec<u8>),
    #[allow(dead_code)]
    Resize { cols: u16, rows: u16 },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
enum Event {
    Replay(Vec<u8>),
    Output(Vec<u8>),
    Exited,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let client = loop {
        match ClientOptions::new().open(PIPE_NAME) {
            Ok(c) => break c,
            Err(e) if e.raw_os_error() == Some(231) => {
                // ERROR_PIPE_BUSY — another client mid-connect, retry shortly.
                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            }
            Err(e) => return Err(e.into()),
        }
    };
    println!("[client] connected to {PIPE_NAME}");

    let (read_half, mut write_half) = tokio::io::split(client);

    let recv_task = tokio::spawn(async move {
        let mut read_half = read_half;
        loop {
            match recv_event(&mut read_half).await {
                Ok(Some(Event::Replay(data))) => {
                    println!("--- replay ({} bytes) ---", data.len());
                    print!("{}", String::from_utf8_lossy(&data));
                }
                Ok(Some(Event::Output(data))) => {
                    print!("{}", String::from_utf8_lossy(&data));
                    use std::io::Write;
                    let _ = std::io::stdout().flush();
                }
                Ok(Some(Event::Exited)) => {
                    println!("\n[client] session exited");
                    break;
                }
                Ok(None) => { println!("[client] daemon closed the connection"); break; }
                Err(e) => { println!("[client] recv error: {e}"); break; }
            }
        }
    });

    let stdin = BufReader::new(tokio::io::stdin());
    let mut lines = stdin.lines();
    while let Ok(Some(line)) = lines.next_line().await {
        if line.trim() == "exit" { break; }
        let mut data = line.into_bytes();
        data.extend_from_slice(b"\r\n");
        send_request(&mut write_half, &Request::Write(data)).await?;
    }

    // Give the daemon a moment to relay the PTY's response before we hang up
    // (stdin closing shouldn't race ahead of output that's still in flight).
    tokio::time::sleep(std::time::Duration::from_millis(1500)).await;

    recv_task.abort();
    Ok(())
}

async fn send_request<W: tokio::io::AsyncWrite + Unpin>(w: &mut W, req: &Request) -> anyhow::Result<()> {
    let bytes = serde_json::to_vec(req)?;
    w.write_u32_le(bytes.len() as u32).await?;
    w.write_all(&bytes).await?;
    w.flush().await?;
    Ok(())
}

async fn recv_event<R: tokio::io::AsyncRead + Unpin>(r: &mut R) -> anyhow::Result<Option<Event>> {
    let len = match r.read_u32_le().await {
        Ok(l) => l,
        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(e) => return Err(e.into()),
    };
    let mut buf = vec![0u8; len as usize];
    r.read_exact(&mut buf).await?;
    Ok(Some(serde_json::from_slice(&buf)?))
}
