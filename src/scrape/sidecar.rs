use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::Duration;

use serde::Deserialize;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};
use tokio::sync::{Mutex, oneshot};
use url::Url;

use super::{ScrapeError, ScrapeErrorKind, Scraper};
use crate::domain::Product;

type Reply = Result<Product, ScrapeError>;

#[derive(Debug, Clone)]
pub struct SidecarCommand {
    pub program: PathBuf,
    pub args: Vec<String>,
    pub cwd: PathBuf,
    pub timeout: Duration,
}

impl SidecarCommand {
    pub fn default_uv(scraper_dir: PathBuf) -> Self {
        Self {
            program: PathBuf::from("uv"),
            args: ["run", "python", "-m", "maeve_scraper"]
                .map(String::from)
                .to_vec(),
            cwd: scraper_dir,
            timeout: Duration::from_secs(60),
        }
    }
}

struct Conn {
    generation: u64,
    child: Child,
    stdin: ChildStdin,
}

struct Pending {
    generation: u64,
    tx: oneshot::Sender<Reply>,
}

#[derive(Default)]
struct Shared {
    conn: Mutex<Option<Conn>>,
    pending: StdMutex<HashMap<u64, Pending>>,
}

impl Shared {
    fn pending(&self) -> std::sync::MutexGuard<'_, HashMap<u64, Pending>> {
        self.pending.lock().unwrap_or_else(|e| e.into_inner())
    }
}

pub struct SidecarScraper {
    cmd: SidecarCommand,
    shared: Arc<Shared>,
    next_id: AtomicU64,
    next_generation: AtomicU64,
}

#[derive(Deserialize)]
struct Wire {
    id: Option<u64>,
    #[serde(default)]
    ok: bool,
    product: Option<Value>,
    error: Option<WireError>,
}

#[derive(Deserialize)]
struct WireError {
    kind: String,
    message: String,
}

fn unavailable(message: impl Into<String>) -> ScrapeError {
    ScrapeError::new(ScrapeErrorKind::SidecarUnavailable, message)
}

fn kind_from_wire(kind: &str) -> ScrapeErrorKind {
    match kind {
        "blocked" => ScrapeErrorKind::Blocked,
        "network" => ScrapeErrorKind::Network,
        "unsupported" => ScrapeErrorKind::Unsupported,
        _ => ScrapeErrorKind::Layout,
    }
}

fn into_reply(wire: Wire) -> Reply {
    if wire.ok {
        let product = wire
            .product
            .ok_or_else(|| ScrapeError::new(ScrapeErrorKind::Layout, "sidecar sent no product"))?;
        return serde_json::from_value(product).map_err(|e| {
            ScrapeError::new(
                ScrapeErrorKind::Layout,
                format!("sidecar product did not match the expected shape: {e}"),
            )
        });
    }
    Err(match wire.error {
        Some(e) => ScrapeError::new(kind_from_wire(&e.kind), e.message),
        None => ScrapeError::new(ScrapeErrorKind::Layout, "sidecar reported an unknown error"),
    })
}

impl SidecarScraper {
    pub fn new(cmd: SidecarCommand) -> Self {
        Self {
            cmd,
            shared: Arc::default(),
            next_id: AtomicU64::new(1),
            next_generation: AtomicU64::new(1),
        }
    }

    fn spawn(&self) -> Result<Conn, ScrapeError> {
        let mut child = Command::new(&self.cmd.program)
            .args(&self.cmd.args)
            .current_dir(&self.cmd.cwd)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .kill_on_drop(true)
            .spawn()
            .map_err(|e| {
                unavailable(format!(
                    "could not start the scraper sidecar `{}` ({e}); install uv (https://docs.astral.sh/uv/) \
                     and run `uv sync` and `uv run scrapling install` in the scraper directory",
                    self.cmd.program.display()
                ))
            })?;
        let (Some(stdin), Some(stdout)) = (child.stdin.take(), child.stdout.take()) else {
            return Err(unavailable("scraper sidecar started without piped stdio"));
        };
        let generation = self.next_generation.fetch_add(1, Ordering::Relaxed);
        tokio::spawn(read_loop(Arc::clone(&self.shared), generation, stdout));
        Ok(Conn {
            generation,
            child,
            stdin,
        })
    }

    pub async fn shutdown(&self) {
        let conn = self.shared.conn.lock().await.take();
        let Some(Conn {
            mut child, stdin, ..
        }) = conn
        else {
            return;
        };
        drop(stdin);
        if tokio::time::timeout(Duration::from_secs(2), child.wait())
            .await
            .is_err()
        {
            let _ = child.kill().await;
        }
    }
}

impl Drop for SidecarScraper {
    fn drop(&mut self) {
        // `read_loop` keeps `Shared` alive, so release the child (stdin EOF + kill_on_drop) here.
        if let Ok(mut conn) = self.shared.conn.try_lock() {
            conn.take();
        }
    }
}

impl Scraper for SidecarScraper {
    async fn scrape(&self, url: Url) -> Result<Product, ScrapeError> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = oneshot::channel();
        let mut line = json!({ "id": id, "url": url.as_str() }).to_string();
        line.push('\n');

        {
            // Register and write under the conn lock so the reader's EOF
            // handling (clear conn, then drain) can never miss this request.
            let timed_out = || {
                ScrapeError::new(
                    ScrapeErrorKind::Timeout,
                    format!(
                        "scraper did not answer within {} s",
                        self.cmd.timeout.as_secs_f32()
                    ),
                )
            };
            let mut guard = tokio::time::timeout(self.cmd.timeout, self.shared.conn.lock())
                .await
                .map_err(|_| timed_out())?;
            if guard.is_none() {
                *guard = Some(self.spawn()?);
            }
            let conn = guard.as_mut().expect("conn set above");
            self.shared.pending().insert(
                id,
                Pending {
                    generation: conn.generation,
                    tx,
                },
            );
            let written = tokio::time::timeout(self.cmd.timeout, async {
                conn.stdin.write_all(line.as_bytes()).await?;
                conn.stdin.flush().await
            })
            .await;
            match written {
                Ok(Ok(())) => {}
                Ok(Err(e)) => {
                    self.shared.pending().remove(&id);
                    *guard = None;
                    return Err(unavailable(format!(
                        "scraper sidecar stopped accepting requests ({e}); it will restart on the next scrape"
                    )));
                }
                Err(_) => {
                    // Wedged sidecar not draining stdin: drop it (kill_on_drop) so the next scrape respawns.
                    self.shared.pending().remove(&id);
                    *guard = None;
                    return Err(timed_out());
                }
            }
        }

        match tokio::time::timeout(self.cmd.timeout, rx).await {
            Ok(Ok(reply)) => reply,
            Ok(Err(_)) => Err(unavailable("scraper sidecar stopped before answering")),
            Err(_) => {
                self.shared.pending().remove(&id);
                Err(ScrapeError::new(
                    ScrapeErrorKind::Timeout,
                    format!(
                        "scraper did not answer within {} s",
                        self.cmd.timeout.as_secs_f32()
                    ),
                ))
            }
        }
    }
}

async fn read_loop(shared: Arc<Shared>, generation: u64, stdout: ChildStdout) {
    let mut lines = BufReader::new(stdout).lines();
    loop {
        let line = match lines.next_line().await {
            Ok(Some(line)) => line,
            Err(e) if e.kind() == std::io::ErrorKind::InvalidData => continue,
            Ok(None) | Err(_) => break,
        };
        // Lines without a numeric id (the `{"ready": true}` banner, stray output) are ignored.
        let Ok(wire) = serde_json::from_str::<Wire>(&line) else {
            continue;
        };
        let Some(id) = wire.id else { continue };
        let pending = {
            let mut map = shared.pending();
            match map.get(&id) {
                Some(p) if p.generation == generation => map.remove(&id),
                _ => None,
            }
        };
        if let Some(p) = pending {
            let _ = p.tx.send(into_reply(wire));
        }
    }

    {
        let mut guard = shared.conn.lock().await;
        if guard.as_ref().is_some_and(|c| c.generation == generation) {
            *guard = None;
        }
    }
    let orphans: Vec<Pending> = {
        let mut map = shared.pending();
        let ids: Vec<u64> = map
            .iter()
            .filter(|(_, p)| p.generation == generation)
            .map(|(id, _)| *id)
            .collect();
        ids.into_iter().filter_map(|id| map.remove(&id)).collect()
    };
    for p in orphans {
        let _ = p.tx.send(Err(unavailable(
            "scraper sidecar exited unexpectedly; it will restart on the next scrape",
        )));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::Site;
    use crate::scrape::classify_url;

    const FAKE: &str = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/fake_sidecar.py"
    );

    fn fake(mode: &str, extra: &[&str], timeout: Duration) -> SidecarScraper {
        let mut args = vec![FAKE.to_string(), mode.to_string()];
        args.extend(extra.iter().map(|s| s.to_string()));
        SidecarScraper::new(SidecarCommand {
            program: PathBuf::from("python3"),
            args,
            cwd: PathBuf::from(env!("CARGO_MANIFEST_DIR")),
            timeout,
        })
    }

    fn temu(n: u32) -> Url {
        classify_url(&format!("https://www.temu.com/item-g-{n}.html"))
            .unwrap()
            .1
    }

    const LONG: Duration = Duration::from_secs(10);

    #[tokio::test]
    async fn scrape_returns_product() {
        let s = fake("echo", &[], LONG);
        let p = s.scrape(temu(1)).await.unwrap();
        assert_eq!(p.site, Site::Temu);
        assert_eq!(p.url, "https://www.temu.com/item-g-1.html");
        s.shutdown().await;
    }

    #[tokio::test]
    async fn out_of_order_responses_reach_their_callers() {
        let s = fake("ooo", &[], LONG);
        let (a, b) = tokio::join!(s.scrape(temu(1)), s.scrape(temu(2)));
        assert_eq!(a.unwrap().url, "https://www.temu.com/item-g-1.html");
        assert_eq!(b.unwrap().url, "https://www.temu.com/item-g-2.html");
        s.shutdown().await;
    }

    #[tokio::test]
    async fn blocked_error_is_mapped() {
        let s = fake("blocked", &[], LONG);
        let err = s.scrape(temu(1)).await.unwrap_err();
        assert_eq!(err.kind, ScrapeErrorKind::Blocked);
        assert_eq!(err.message, "captcha");
        s.shutdown().await;
    }

    #[tokio::test]
    async fn missing_program_is_sidecar_unavailable_and_mentions_uv() {
        let s = SidecarScraper::new(SidecarCommand {
            program: PathBuf::from("/nonexistent/uv"),
            args: vec![],
            cwd: PathBuf::from(env!("CARGO_MANIFEST_DIR")),
            timeout: LONG,
        });
        let err = s.scrape(temu(1)).await.unwrap_err();
        assert_eq!(err.kind, ScrapeErrorKind::SidecarUnavailable);
        assert!(err.message.contains("uv"), "{}", err.message);
    }

    #[tokio::test]
    async fn crash_fails_pending_then_respawns() {
        let marker = std::env::temp_dir().join(format!(
            "maeve-fake-sidecar-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = std::fs::remove_file(&marker);
        let s = fake("crash_once", &[marker.to_str().unwrap()], LONG);

        let err = s.scrape(temu(1)).await.unwrap_err();
        assert_eq!(err.kind, ScrapeErrorKind::SidecarUnavailable);

        let p = s.scrape(temu(2)).await.unwrap();
        assert_eq!(p.url, "https://www.temu.com/item-g-2.html");
        s.shutdown().await;
        let _ = std::fs::remove_file(&marker);
    }

    #[tokio::test]
    async fn silent_sidecar_times_out() {
        let s = fake("silent", &[], Duration::from_millis(200));
        let err = s.scrape(temu(1)).await.unwrap_err();
        assert_eq!(err.kind, ScrapeErrorKind::Timeout);
        assert!(s.shared.pending().is_empty());
        s.shutdown().await;
    }
}
