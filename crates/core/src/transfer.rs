//! Moving file bytes between peers: the request pipeline on the receiving
//! side, block serving on the sending side, and the progress the interface
//! shows.

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};
use bytes::Bytes;
use tokio::fs::File;
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt};
use tokio::sync::oneshot;
use tokio::task::JoinSet;

use crate::clock::mtime_ms;
use crate::conn::Connection;
use crate::hash::blake3_file;
use crate::index::Entry;
use crate::paths::{parent_of, safe_abs};
use crate::proto::{Control, Frame, BLOCK_OK, BLOCK_SIZE, BLOCK_UNAVAILABLE, BLOCK_WINDOW};
use crate::state::{Transfer, TransferDirection, TransferSummary};

/// Outstanding block requests per download.
const WINDOW: usize = BLOCK_WINDOW;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
const RATE_WINDOW: Duration = Duration::from_secs(2);
/// An upload with no request for this long is over.
const UPLOAD_IDLE: Duration = Duration::from_secs(2);

pub type SharedTransfers = Arc<Mutex<TransferState>>;

struct Progress {
    done: u64,
    total: u64,
}

struct Upload {
    done: u64,
    total: u64,
    last: Instant,
}

/// Progress of every transfer, shared between download tasks, request
/// handlers and the state builder.
#[derive(Default)]
pub struct TransferState {
    downloads: HashMap<(String, String), Progress>,
    uploads: HashMap<(String, String), Upload>,
    samples: VecDeque<(Instant, u64)>,
    /// Every byte moved over the network in either direction, ever.
    total_bytes: u64,
}

impl TransferState {
    pub fn begin_download(&mut self, peer: &str, path: &str, total: u64) {
        self.downloads
            .insert((peer.into(), path.into()), Progress { done: 0, total });
    }

    pub fn download_progress(&mut self, peer: &str, path: &str, added: u64) {
        if let Some(p) = self.downloads.get_mut(&(peer.into(), path.into())) {
            p.done += added;
        }
        self.note_bytes(added);
    }

    pub fn end_download(&mut self, peer: &str, path: &str) {
        self.downloads.remove(&(peer.into(), path.into()));
    }

    pub fn note_upload(&mut self, peer: &str, path: &str, added: u64, total: u64) {
        let key = (peer.to_string(), path.to_string());
        let up = self.uploads.entry(key).or_insert(Upload {
            done: 0,
            total,
            last: Instant::now(),
        });
        up.done += added;
        up.total = total;
        up.last = Instant::now();
        self.note_bytes(added);
    }

    /// Fraction done if any transfer for the path is active.
    pub fn progress_for(&self, path: &str) -> Option<f32> {
        let fraction = |done: u64, total: u64| {
            if total == 0 {
                1.0
            } else {
                (done as f64 / total as f64).min(1.0) as f32
            }
        };
        if let Some(p) = self.downloads.iter().find(|(k, _)| k.1 == path) {
            return Some(fraction(p.1.done, p.1.total));
        }
        let now = Instant::now();
        self.uploads
            .iter()
            .find(|(k, u)| k.1 == path && u.done < u.total && now - u.last < UPLOAD_IDLE)
            .map(|(_, u)| fraction(u.done, u.total))
    }

    /// Bytes moved over the network so far, in either direction. Local
    /// copies never count, which is what makes it a useful measure.
    pub fn total_bytes(&self) -> u64 {
        self.total_bytes
    }

    fn note_bytes(&mut self, n: u64) {
        self.total_bytes += n;
        let now = Instant::now();
        self.samples.push_back((now, n));
        while let Some((t, _)) = self.samples.front() {
            if now - *t > RATE_WINDOW {
                self.samples.pop_front();
            } else {
                break;
            }
        }
    }

    /// `queued` is how many downloads wait on the links, which the engine
    /// knows and this state does not.
    pub fn summary(&mut self, queued: u32) -> TransferSummary {
        let now = Instant::now();
        self.uploads
            .retain(|_, u| u.done < u.total && now - u.last < UPLOAD_IDLE);
        while let Some((t, _)) = self.samples.front() {
            if now - *t > RATE_WINDOW {
                self.samples.pop_front();
            } else {
                break;
            }
        }
        let mut active: Vec<Transfer> = self
            .downloads
            .iter()
            .map(|((peer, path), p)| Transfer {
                path: path.clone(),
                peer_id: peer.clone(),
                direction: TransferDirection::Download,
                bytes_done: p.done,
                bytes_total: p.total,
            })
            .chain(self.uploads.iter().map(|((peer, path), u)| Transfer {
                path: path.clone(),
                peer_id: peer.clone(),
                direction: TransferDirection::Upload,
                bytes_done: u.done,
                bytes_total: u.total,
            }))
            .collect();
        active.sort_by(|a, b| a.path.cmp(&b.path));
        let bytes: u64 = self.samples.iter().map(|(_, n)| *n).sum();
        TransferSummary {
            active,
            queued,
            bytes_per_sec: bytes / RATE_WINDOW.as_secs(),
        }
    }
}

/// Issues block requests on one connection and routes the answers back.
pub struct Requester {
    conn: Connection,
    next_id: AtomicU64,
    pending: Mutex<HashMap<u64, oneshot::Sender<(u8, Bytes)>>>,
}

impl Requester {
    pub fn new(conn: Connection) -> Arc<Requester> {
        Arc::new(Requester {
            conn,
            next_id: AtomicU64::new(1),
            pending: Mutex::new(HashMap::new()),
        })
    }

    pub fn conn(&self) -> &Connection {
        &self.conn
    }

    pub async fn request(
        &self,
        path: &str,
        hash: &str,
        offset: u64,
        len: u32,
    ) -> Result<(u8, Bytes)> {
        let req_id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = oneshot::channel();
        self.pending
            .lock()
            .expect("requester lock")
            .insert(req_id, tx);
        let frame = Frame::Control(Control::Request {
            req_id,
            path: path.to_string(),
            hash: hash.to_string(),
            offset,
            len,
        });
        if let Err(e) = self.conn.send(frame).await {
            self.pending.lock().expect("requester lock").remove(&req_id);
            return Err(e);
        }
        match tokio::time::timeout(REQUEST_TIMEOUT, rx).await {
            Ok(Ok(answer)) => Ok(answer),
            Ok(Err(_)) => bail!("connection closed while waiting for a block"),
            Err(_) => {
                self.pending.lock().expect("requester lock").remove(&req_id);
                bail!("block request timed out")
            }
        }
    }

    /// Hands a received block to the waiting request, if it is still waited for.
    pub fn deliver(&self, req_id: u64, status: u8, data: Bytes) {
        if let Some(tx) = self.pending.lock().expect("requester lock").remove(&req_id) {
            let _ = tx.send((status, data));
        }
    }

    /// Fails every outstanding request, for a closed connection.
    pub fn fail_all(&self) {
        self.pending.lock().expect("requester lock").clear();
    }
}

/// `.owl-tmp-<hash prefix>-<random>` beside the target, so a rename into
/// place is atomic and the scanner never indexes it. The directory is
/// checked for symbolic links and only alphanumerics of the hash are used,
/// so nothing a peer sends can steer the name.
pub fn tmp_path(root: &Path, entry: &Entry) -> Result<PathBuf> {
    let dir = safe_abs(root, parent_of(&entry.path))?;
    let prefix: String = entry
        .hash
        .as_deref()
        .unwrap_or("")
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .take(16)
        .collect();
    let prefix = if prefix.is_empty() {
        "nohash".to_string()
    } else {
        prefix
    };
    let suffix: u32 = rand::random();
    Ok(dir.join(format!(".owl-tmp-{prefix}-{suffix:08x}")))
}

/// Fetches a file's bytes from the peer into a temporary file beside the
/// target and verifies the whole-file hash. Returns the temporary path;
/// the caller sets the mtime and renames it into place. The temporary
/// file keeps a fresh mtime until then, so the stale-file sweep never
/// mistakes it for a leftover.
pub async fn download(
    req: Arc<Requester>,
    root: &Path,
    entry: &Entry,
    transfers: SharedTransfers,
) -> Result<PathBuf> {
    let hash = entry.hash.clone().context("a file entry carries a hash")?;
    let tmp = tmp_path(root, entry)?;
    if let Some(parent) = tmp.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    let mut file = File::create(&tmp)
        .await
        .with_context(|| format!("creating {}", tmp.display()))?;
    let peer = req.conn().peer_id.clone();
    transfers
        .lock()
        .expect("transfers lock")
        .begin_download(&peer, &entry.path, entry.size);
    let result = download_blocks(
        &req,
        &entry.path,
        &hash,
        entry.size,
        &mut file,
        &transfers,
        &peer,
    )
    .await;
    transfers
        .lock()
        .expect("transfers lock")
        .end_download(&peer, &entry.path);
    let outcome = match result {
        Ok(actual) => {
            file.flush().await?;
            drop(file);
            if actual != hash {
                Err(anyhow::anyhow!(
                    "downloaded {} does not match the announced hash",
                    entry.path
                ))
            } else {
                Ok(())
            }
        }
        Err(e) => {
            drop(file);
            Err(e)
        }
    };
    match outcome {
        Ok(()) => Ok(tmp),
        Err(e) => {
            let _ = tokio::fs::remove_file(&tmp).await;
            Err(e)
        }
    }
}

async fn download_blocks(
    req: &Arc<Requester>,
    path: &str,
    hash: &str,
    size: u64,
    file: &mut File,
    transfers: &SharedTransfers,
    peer: &str,
) -> Result<String> {
    let mut hasher = blake3::Hasher::new();
    let mut next_request: u64 = 0;
    let mut next_write: u64 = 0;
    let mut inflight: JoinSet<(u64, Result<(u8, Bytes)>)> = JoinSet::new();
    let mut ready: BTreeMap<u64, Bytes> = BTreeMap::new();
    while next_write < size {
        while inflight.len() + ready.len() < WINDOW && next_request < size {
            let len = (size - next_request).min(BLOCK_SIZE as u64) as u32;
            let offset = next_request;
            let req = req.clone();
            let path = path.to_string();
            let hash = hash.to_string();
            inflight.spawn(async move { (offset, req.request(&path, &hash, offset, len).await) });
            next_request += len as u64;
        }
        let Some(joined) = inflight.join_next().await else {
            bail!("download of {path} stalled");
        };
        let (offset, res) = joined.context("block task failed")?;
        let (status, data) = res?;
        if status != BLOCK_OK {
            bail!("peer no longer has {path} at the announced version");
        }
        let expected = (size - offset).min(BLOCK_SIZE as u64) as usize;
        if data.len() != expected {
            bail!("short block for {path}: {} of {expected} bytes", data.len());
        }
        ready.insert(offset, data);
        while let Some(data) = ready.remove(&next_write) {
            file.write_all(&data).await?;
            hasher.update(&data);
            next_write += data.len() as u64;
            transfers.lock().expect("transfers lock").download_progress(
                peer,
                path,
                data.len() as u64,
            );
        }
    }
    Ok(hasher.finalize().to_hex().to_string())
}

/// Copies a local file with the wanted hash to a temporary path beside the
/// target and verifies it. `None` means the source no longer matches and
/// the bytes must come from the network.
pub async fn local_copy(root: &Path, source_rel: &str, entry: &Entry) -> Result<Option<PathBuf>> {
    let source = safe_abs(root, source_rel)?;
    let tmp = tmp_path(root, entry)?;
    if let Some(parent) = tmp.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    if let Err(e) = tokio::fs::copy(&source, &tmp).await {
        let _ = tokio::fs::remove_file(&tmp).await;
        return Err(e.into());
    }
    let hash = blake3_file(&tmp).await?;
    if Some(hash) != entry.hash {
        let _ = tokio::fs::remove_file(&tmp).await;
        return Ok(None);
    }
    Ok(Some(tmp))
}

/// What the index says about a file we may serve.
#[derive(Clone, Debug)]
pub struct Served {
    pub hash: String,
    pub size: u64,
    pub mtime_ms: i64,
}

/// Answers a block request. Unavailable when the index or the file on disk
/// no longer matches what the peer asked for.
pub async fn serve_request(
    root: &Path,
    served: Option<Served>,
    req_id: u64,
    path: &str,
    hash: &str,
    offset: u64,
    len: u32,
) -> Frame {
    let unavailable = Frame::Block {
        req_id,
        status: BLOCK_UNAVAILABLE,
        data: Bytes::new(),
    };
    let Some(served) = served else {
        return unavailable;
    };
    let end = offset.checked_add(len as u64);
    if served.hash != hash || len > BLOCK_SIZE || end.is_none_or(|end| end > served.size) {
        return unavailable;
    }
    let Ok(abs) = safe_abs(root, path) else {
        return unavailable;
    };
    let Ok(md) = tokio::fs::metadata(&abs).await else {
        return unavailable;
    };
    if md.len() != served.size || mtime_ms(&md) != served.mtime_ms {
        return unavailable;
    }
    let read = async {
        let mut file = File::open(&abs).await?;
        file.seek(std::io::SeekFrom::Start(offset)).await?;
        let mut buf = vec![0u8; len as usize];
        file.read_exact(&mut buf).await?;
        Ok::<_, std::io::Error>(buf)
    };
    match read.await {
        Ok(buf) => Frame::Block {
            req_id,
            status: BLOCK_OK,
            data: Bytes::from(buf),
        },
        Err(_) => unavailable,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hash::blake3_hex;
    use crate::index::EntryKind;
    use std::collections::BTreeMap;

    fn entry(path: &str, data: &[u8], mtime: i64) -> Entry {
        Entry {
            path: path.into(),
            kind: EntryKind::File,
            size: data.len() as u64,
            mtime_ms: mtime,
            hash: Some(blake3_hex(data)),
            deleted: false,
            vv: BTreeMap::new(),
            seen_at_ms: 0,
        }
    }

    #[test]
    fn tmp_path_is_ignored_and_beside_the_target() {
        let dir = tempfile::tempdir().unwrap();
        let e = entry("docs/a.txt", b"x", 0);
        let p = tmp_path(dir.path(), &e).unwrap();
        assert_eq!(p.parent(), Some(dir.path().join("docs").as_path()));
        let name = p.file_name().unwrap().to_str().unwrap();
        assert!(name.starts_with(".owl-tmp-"));
        assert!(crate::ignore::is_ignored(&format!("docs/{name}")));
    }

    #[tokio::test]
    async fn hostile_hash_cannot_escape_or_panic() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("root");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("src.bin"), b"bytes").unwrap();

        let mut escaping = entry("x.bin", b"bytes", 0);
        escaping.hash = Some("../../../escape/".to_string());
        let p = tmp_path(&root, &escaping).unwrap();
        assert_eq!(p.parent(), Some(root.as_path()));
        assert!(p
            .file_name()
            .unwrap()
            .to_str()
            .unwrap()
            .starts_with(".owl-tmp-escape-"));
        assert!(local_copy(&root, "src.bin", &escaping)
            .await
            .unwrap()
            .is_none());
        assert!(!dir.path().join("escape").exists());
        assert!(!root.join(".owl-tmp-..").exists());

        let mut wide = entry("y.bin", b"bytes", 0);
        wide.hash = Some("\u{20ac}".repeat(6));
        let p = tmp_path(&root, &wide).unwrap();
        assert!(p
            .file_name()
            .unwrap()
            .to_str()
            .unwrap()
            .starts_with(".owl-tmp-nohash-"));
        assert!(local_copy(&root, "src.bin", &wide).await.unwrap().is_none());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_symbolic_link_in_the_path_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("root");
        let outside = dir.path().join("outside");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        std::os::unix::fs::symlink(&outside, root.join("link")).unwrap();
        std::fs::write(root.join("src.bin"), b"bytes").unwrap();

        let through_link = entry("link/escaped.bin", b"bytes", 0);
        assert!(tmp_path(&root, &through_link).is_err());
        assert!(local_copy(&root, "src.bin", &through_link).await.is_err());
        assert!(local_copy(&root, "link/whatever", &through_link)
            .await
            .is_err());
        let served = Served {
            hash: blake3_hex(b"bytes"),
            size: 5,
            mtime_ms: 0,
        };
        let answer = serve_request(
            &root,
            Some(served.clone()),
            1,
            "link/escaped.bin",
            &served.hash,
            0,
            5,
        )
        .await;
        assert!(matches!(
            answer,
            Frame::Block {
                status: BLOCK_UNAVAILABLE,
                ..
            }
        ));
        assert!(std::fs::read_dir(&outside).unwrap().next().is_none());
    }

    #[tokio::test]
    async fn local_copy_verifies_the_hash_and_keeps_a_fresh_mtime() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("src.bin"), b"same bytes").unwrap();
        let wanted = entry("copy.bin", b"same bytes", 1_700_000_000_123);
        let tmp = local_copy(dir.path(), "src.bin", &wanted)
            .await
            .unwrap()
            .expect("hashes match");
        assert_eq!(std::fs::read(&tmp).unwrap(), b"same bytes");
        // The entry's mtime is applied at install time, not here, so the
        // temporary file cannot look stale while it waits.
        let age = crate::clock::now_ms() - mtime_ms(&std::fs::metadata(&tmp).unwrap());
        assert!(age.abs() < 60_000, "age {age} ms");

        let other = entry("copy2.bin", b"different", 1);
        assert!(local_copy(dir.path(), "src.bin", &other)
            .await
            .unwrap()
            .is_none());
        let leftovers: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_str().unwrap().starts_with(".owl-tmp-"))
            .collect();
        assert_eq!(leftovers.len(), 1, "only the good copy remains");
    }

    #[tokio::test]
    async fn serve_request_checks_index_and_disk() {
        let dir = tempfile::tempdir().unwrap();
        let data: Vec<u8> = (0..1000u32).map(|i| i as u8).collect();
        std::fs::write(dir.path().join("f.bin"), &data).unwrap();
        let md = std::fs::metadata(dir.path().join("f.bin")).unwrap();
        let served = Served {
            hash: blake3_hex(&data),
            size: 1000,
            mtime_ms: mtime_ms(&md),
        };
        let ok = serve_request(
            dir.path(),
            Some(served.clone()),
            1,
            "f.bin",
            &served.hash,
            500,
            500,
        )
        .await;
        match ok {
            Frame::Block {
                req_id,
                status,
                data: got,
            } => {
                assert_eq!(req_id, 1);
                assert_eq!(status, BLOCK_OK);
                assert_eq!(&got[..], &data[500..]);
            }
            other => panic!("unexpected {other:?}"),
        }
        let wrong_hash =
            serve_request(dir.path(), Some(served.clone()), 2, "f.bin", "nope", 0, 10).await;
        assert!(matches!(
            wrong_hash,
            Frame::Block {
                status: BLOCK_UNAVAILABLE,
                ..
            }
        ));
        let past_end = serve_request(
            dir.path(),
            Some(served.clone()),
            3,
            "f.bin",
            &served.hash,
            900,
            200,
        )
        .await;
        assert!(matches!(
            past_end,
            Frame::Block {
                status: BLOCK_UNAVAILABLE,
                ..
            }
        ));
        let wrapped = serve_request(
            dir.path(),
            Some(served.clone()),
            5,
            "f.bin",
            &served.hash,
            u64::MAX - 10,
            100,
        )
        .await;
        assert!(matches!(
            wrapped,
            Frame::Block {
                status: BLOCK_UNAVAILABLE,
                ..
            }
        ));
        let unknown = serve_request(dir.path(), None, 4, "f.bin", &served.hash, 0, 10).await;
        assert!(matches!(
            unknown,
            Frame::Block {
                status: BLOCK_UNAVAILABLE,
                ..
            }
        ));
    }

    #[test]
    fn summary_reports_active_queued_and_rate() {
        let mut t = TransferState::default();
        t.begin_download("peer", "a.bin", 1000);
        t.download_progress("peer", "a.bin", 400);
        t.note_upload("peer", "b.bin", 100, 1000);
        let s = t.summary(2);
        assert_eq!(s.queued, 2);
        assert_eq!(s.bytes_per_sec, 250);
        assert_eq!(s.active.len(), 2);
        assert_eq!(s.active[0].path, "a.bin");
        assert_eq!(s.active[0].direction, TransferDirection::Download);
        assert_eq!(s.active[0].bytes_done, 400);
        assert_eq!(s.active[1].direction, TransferDirection::Upload);
        assert_eq!(t.progress_for("a.bin"), Some(0.4));
        assert_eq!(t.progress_for("b.bin"), Some(0.1));
        assert_eq!(t.progress_for("c.bin"), None);
        t.end_download("peer", "a.bin");
        t.note_upload("peer", "b.bin", 900, 1000);
        let s = t.summary(0);
        assert!(s.active.is_empty(), "finished uploads are pruned");
        assert_eq!(s.queued, 0);
        assert_eq!(t.total_bytes(), 1400);
    }
}
