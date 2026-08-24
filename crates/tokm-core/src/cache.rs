//! Private content-addressed count store and conservative path metadata index.

use std::{
    fs::{self, File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use directories::ProjectDirs;
use fs2::FileExt;
use path_clean::PathClean;
use serde::{Deserialize, Serialize, de::DeserializeOwned};

use crate::{COUNTING_SEMANTICS_VERSION, CountOptions, InvalidUtf8Policy};

const CACHE_FORMAT_VERSION: u32 = 1;
const MAX_CACHE_ENTRY_BYTES: u64 = 64 * 1024;
const RACY_MTIME_WINDOW_NANOS: u64 = 2_000_000_000;
static TEMPORARY_FILE_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Debug)]
pub(crate) enum CachePolicy {
    Default,
    Disabled,
    Directory(PathBuf),
}

#[derive(Clone, Debug)]
pub(crate) struct Cache {
    root: PathBuf,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct MetadataSnapshot {
    size: u64,
    modified_nanos: Option<u64>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct CachedCount {
    pub(crate) tokens: u64,
    pub(crate) decoded_lossily: bool,
}

#[derive(Debug, Deserialize, Serialize)]
struct CountEntry {
    format_version: u32,
    key: String,
    tokens: u64,
    decoded_lossily: bool,
    integrity: String,
}

#[derive(Debug, Deserialize, Serialize)]
struct MetadataEntry {
    format_version: u32,
    path_fingerprint: String,
    size: u64,
    modified_nanos: u64,
    checkpoint_nanos: u64,
    content_hash: String,
    integrity: String,
}

impl Cache {
    pub(crate) fn resolve(policy: &CachePolicy) -> Option<Self> {
        let root = match policy {
            CachePolicy::Disabled => return None,
            CachePolicy::Directory(path) => path.clone(),
            CachePolicy::Default => ProjectDirs::from("", "", "tokm")?.cache_dir().to_path_buf(),
        };

        Some(Self { root })
    }

    pub(crate) fn content_hash(bytes: &[u8]) -> String {
        blake3::hash(bytes).to_hex().to_string()
    }

    pub(crate) fn metadata_snapshot(metadata: &fs::Metadata) -> MetadataSnapshot {
        MetadataSnapshot {
            size: metadata.len(),
            modified_nanos: metadata.modified().ok().and_then(system_time_nanos),
        }
    }

    pub(crate) fn lookup_content_hash(
        &self,
        path: &Path,
        snapshot: MetadataSnapshot,
    ) -> Option<String> {
        let path_fingerprint = path_fingerprint(path)?;
        let entry: MetadataEntry = read_json(&self.metadata_path(&path_fingerprint))?;

        entry
            .is_usable(&path_fingerprint, snapshot)
            .then_some(entry.content_hash)
    }

    pub(crate) fn store_content_hash(
        &self,
        path: &Path,
        snapshot: MetadataSnapshot,
        content_hash: &str,
    ) {
        let Some(path_fingerprint) = path_fingerprint(path) else {
            return;
        };
        let Some(modified_nanos) = snapshot.modified_nanos else {
            return;
        };
        let Some(checkpoint_nanos) = system_time_nanos(SystemTime::now()) else {
            return;
        };
        let entry = MetadataEntry::new(
            path_fingerprint.clone(),
            snapshot.size,
            modified_nanos,
            checkpoint_nanos,
            content_hash.to_owned(),
        );
        let path = self.metadata_path(&path_fingerprint);
        let lock_path = path.with_extension("lock");

        let _ = with_exclusive_lock(&lock_path, || {
            if read_json::<MetadataEntry>(&path).is_some_and(|current| {
                current.is_intact() && current.checkpoint_nanos > checkpoint_nanos
            }) {
                return Ok(());
            }

            write_json_atomic(&path, &entry)
        });
    }

    pub(crate) fn lookup_count(
        &self,
        content_hash: &str,
        options: &CountOptions,
        invalid_utf8: InvalidUtf8Policy,
    ) -> Option<CachedCount> {
        let key = count_key(content_hash, options, invalid_utf8);
        let entry: CountEntry = read_json(&self.count_path(&key))?;
        if !entry.is_valid_for(&key) {
            return None;
        }

        Some(CachedCount {
            tokens: entry.tokens,
            decoded_lossily: entry.decoded_lossily,
        })
    }

    pub(crate) fn store_count(
        &self,
        content_hash: &str,
        options: &CountOptions,
        invalid_utf8: InvalidUtf8Policy,
        count: CachedCount,
    ) {
        let key = count_key(content_hash, options, invalid_utf8);
        let path = self.count_path(&key);
        let lock_path = path.with_extension("lock");
        let entry = CountEntry::new(key.clone(), count);

        let _ = with_exclusive_lock(&lock_path, || {
            if read_json::<CountEntry>(&path).is_some_and(|current| current.is_valid_for(&key)) {
                return Ok(());
            }

            write_json_atomic(&path, &entry)
        });
    }

    fn count_path(&self, key: &str) -> PathBuf {
        self.root
            .join("counts")
            .join(CACHE_FORMAT_VERSION.to_string())
            .join(&key[..2])
            .join(format!("{key}.json"))
    }

    fn metadata_path(&self, path_fingerprint: &str) -> PathBuf {
        self.root
            .join("index")
            .join(CACHE_FORMAT_VERSION.to_string())
            .join(&path_fingerprint[..2])
            .join(format!("{path_fingerprint}.json"))
    }
}

impl CountEntry {
    fn new(key: String, count: CachedCount) -> Self {
        let integrity = count_entry_integrity(&key, count.tokens, count.decoded_lossily);
        Self {
            format_version: CACHE_FORMAT_VERSION,
            key,
            tokens: count.tokens,
            decoded_lossily: count.decoded_lossily,
            integrity,
        }
    }

    fn is_valid_for(&self, key: &str) -> bool {
        self.format_version == CACHE_FORMAT_VERSION
            && self.key == key
            && self.integrity == count_entry_integrity(&self.key, self.tokens, self.decoded_lossily)
    }
}

impl MetadataEntry {
    fn new(
        path_fingerprint: String,
        size: u64,
        modified_nanos: u64,
        checkpoint_nanos: u64,
        content_hash: String,
    ) -> Self {
        let integrity = metadata_entry_integrity(
            &path_fingerprint,
            size,
            modified_nanos,
            checkpoint_nanos,
            &content_hash,
        );
        Self {
            format_version: CACHE_FORMAT_VERSION,
            path_fingerprint,
            size,
            modified_nanos,
            checkpoint_nanos,
            content_hash,
            integrity,
        }
    }

    fn is_intact(&self) -> bool {
        self.format_version == CACHE_FORMAT_VERSION
            && self.integrity
                == metadata_entry_integrity(
                    &self.path_fingerprint,
                    self.size,
                    self.modified_nanos,
                    self.checkpoint_nanos,
                    &self.content_hash,
                )
    }

    fn is_usable(&self, path_fingerprint: &str, snapshot: MetadataSnapshot) -> bool {
        let Some(modified_nanos) = snapshot.modified_nanos else {
            return false;
        };

        self.is_intact()
            && self.path_fingerprint == path_fingerprint
            && self.size == snapshot.size
            && self.modified_nanos == modified_nanos
            && modified_nanos
                .checked_add(RACY_MTIME_WINDOW_NANOS)
                .is_some_and(|safe_before| safe_before < self.checkpoint_nanos)
    }
}

fn count_entry_integrity(key: &str, tokens: u64, decoded_lossily: bool) -> String {
    let mut hasher = blake3::Hasher::new();
    hash_component(&mut hasher, b"tokm-cache-count-entry-v1");
    hash_component(&mut hasher, key.as_bytes());
    hash_component(&mut hasher, &tokens.to_le_bytes());
    hash_component(&mut hasher, &[u8::from(decoded_lossily)]);
    hasher.finalize().to_hex().to_string()
}

fn metadata_entry_integrity(
    path_fingerprint: &str,
    size: u64,
    modified_nanos: u64,
    checkpoint_nanos: u64,
    content_hash: &str,
) -> String {
    let mut hasher = blake3::Hasher::new();
    hash_component(&mut hasher, b"tokm-cache-metadata-entry-v1");
    hash_component(&mut hasher, path_fingerprint.as_bytes());
    hash_component(&mut hasher, &size.to_le_bytes());
    hash_component(&mut hasher, &modified_nanos.to_le_bytes());
    hash_component(&mut hasher, &checkpoint_nanos.to_le_bytes());
    hash_component(&mut hasher, content_hash.as_bytes());
    hasher.finalize().to_hex().to_string()
}

fn count_key(
    content_hash: &str,
    options: &CountOptions,
    invalid_utf8: InvalidUtf8Policy,
) -> String {
    let mut hasher = blake3::Hasher::new();
    hash_component(&mut hasher, b"tokm-count-key-v1");
    hash_component(&mut hasher, content_hash.as_bytes());
    hash_component(
        &mut hasher,
        options.tokenizer().metadata().fingerprint.as_bytes(),
    );
    hash_component(&mut hasher, options.text_policy().fingerprint().as_bytes());
    hash_component(&mut hasher, invalid_utf8.fingerprint().as_bytes());
    hash_component(&mut hasher, &COUNTING_SEMANTICS_VERSION.to_le_bytes());
    hasher.finalize().to_hex().to_string()
}

fn hash_component(hasher: &mut blake3::Hasher, bytes: &[u8]) {
    hasher.update(&(bytes.len() as u64).to_le_bytes());
    hasher.update(bytes);
}

fn path_fingerprint(path: &Path) -> Option<String> {
    let absolute = if path.is_absolute() {
        path.clean()
    } else {
        std::env::current_dir().ok()?.join(path).clean()
    };
    let mut hasher = blake3::Hasher::new();
    hash_path_bytes(&mut hasher, &absolute);
    Some(hasher.finalize().to_hex().to_string())
}

#[cfg(unix)]
fn hash_path_bytes(hasher: &mut blake3::Hasher, path: &Path) {
    use std::os::unix::ffi::OsStrExt;

    hasher.update(path.as_os_str().as_bytes());
}

#[cfg(windows)]
fn hash_path_bytes(hasher: &mut blake3::Hasher, path: &Path) {
    use std::os::windows::ffi::OsStrExt;

    for unit in path.as_os_str().encode_wide() {
        hasher.update(&unit.to_le_bytes());
    }
}

#[cfg(not(any(unix, windows)))]
fn hash_path_bytes(hasher: &mut blake3::Hasher, path: &Path) {
    hasher.update(path.as_os_str().to_string_lossy().as_bytes());
}

fn system_time_nanos(time: SystemTime) -> Option<u64> {
    let nanos = time.duration_since(UNIX_EPOCH).ok()?.as_nanos();
    u64::try_from(nanos).ok()
}

fn read_json<T: DeserializeOwned>(path: &Path) -> Option<T> {
    let metadata = fs::metadata(path).ok()?;
    if metadata.len() > MAX_CACHE_ENTRY_BYTES {
        return None;
    }

    let file = File::open(path).ok()?;
    serde_json::from_reader(file).ok()
}

fn with_exclusive_lock(
    path: &Path,
    operation: impl FnOnce() -> std::io::Result<()>,
) -> std::io::Result<()> {
    let Some(parent) = path.parent() else {
        return Err(std::io::Error::other("cache lock has no parent"));
    };
    fs::create_dir_all(parent)?;
    let lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(path)?;
    lock.lock_exclusive()?;

    operation()
}

fn write_json_atomic(path: &Path, value: &impl Serialize) -> std::io::Result<()> {
    let Some(parent) = path.parent() else {
        return Err(std::io::Error::other("cache entry has no parent"));
    };
    fs::create_dir_all(parent)?;
    let sequence = TEMPORARY_FILE_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("entry");
    let temporary = parent.join(format!(
        ".{file_name}.{}.{}.tmp",
        std::process::id(),
        sequence
    ));
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&temporary)?;
    serde_json::to_writer(&mut file, value).map_err(std::io::Error::other)?;
    file.write_all(b"\n")?;
    file.sync_all()?;
    drop(file);

    if let Err(first_error) = fs::rename(&temporary, path) {
        if path.is_file() {
            fs::remove_file(path)?;
            if let Err(second_error) = fs::rename(&temporary, path) {
                let _ = fs::remove_file(&temporary);
                return Err(second_error);
            }
        } else {
            let _ = fs::remove_file(&temporary);
            return Err(first_error);
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Barrier};

    use tempfile::tempdir;

    use super::{
        Cache, CachedCount, CountEntry, MetadataEntry, MetadataSnapshot, RACY_MTIME_WINDOW_NANOS,
        count_key, write_json_atomic,
    };
    use crate::{CountOptions, InvalidUtf8Policy, TextPolicy, tokenizer::BuiltinEncoding};

    #[test]
    fn potentially_racy_metadata_is_never_trusted() {
        let snapshot = MetadataSnapshot {
            size: 4,
            modified_nanos: Some(10),
        };
        let entry = MetadataEntry::new(
            "path".to_owned(),
            4,
            10,
            10 + RACY_MTIME_WINDOW_NANOS,
            "content".to_owned(),
        );

        assert!(!entry.is_usable("path", snapshot));

        let safe_entry = MetadataEntry::new(
            "path".to_owned(),
            4,
            10,
            11 + RACY_MTIME_WINDOW_NANOS,
            "content".to_owned(),
        );
        assert!(safe_entry.is_usable("path", snapshot));

        let mut corrupt_entry = safe_entry;
        corrupt_entry.content_hash = "different".to_owned();
        assert!(!corrupt_entry.is_usable("path", snapshot));
    }

    #[test]
    fn concurrent_publishers_converge_on_one_count() {
        let directory = tempdir().unwrap();
        let cache = Arc::new(Cache {
            root: directory.path().to_path_buf(),
        });
        let barrier = Arc::new(Barrier::new(8));
        let content_hash = Cache::content_hash(b"same content");
        let options = Arc::new(CountOptions::default());
        let threads = (0..8)
            .map(|_| {
                let cache = Arc::clone(&cache);
                let barrier = Arc::clone(&barrier);
                let content_hash = content_hash.clone();
                let options = Arc::clone(&options);

                std::thread::spawn(move || {
                    barrier.wait();
                    cache.store_count(
                        &content_hash,
                        &options,
                        InvalidUtf8Policy::Skip,
                        CachedCount {
                            tokens: 2,
                            decoded_lossily: false,
                        },
                    );
                })
            })
            .collect::<Vec<_>>();

        for thread in threads {
            thread.join().unwrap();
        }

        assert_eq!(
            cache.lookup_count(&content_hash, &options, InvalidUtf8Policy::Skip),
            Some(CachedCount {
                tokens: 2,
                decoded_lossily: false
            })
        );
    }

    #[test]
    fn count_key_covers_every_counting_semantic() {
        let default = CountOptions::default();
        let normalized = default.clone().with_text_policy(TextPolicy::Normalized);
        let other_encoding = CountOptions::for_encoding(BuiltinEncoding::Cl100kBase);

        let baseline = count_key("content", &default, InvalidUtf8Policy::Skip);

        assert_ne!(
            baseline,
            count_key("other-content", &default, InvalidUtf8Policy::Skip)
        );
        assert_ne!(
            baseline,
            count_key("content", &normalized, InvalidUtf8Policy::Skip)
        );
        assert_ne!(
            baseline,
            count_key("content", &other_encoding, InvalidUtf8Policy::Skip)
        );
        assert_ne!(
            baseline,
            count_key("content", &default, InvalidUtf8Policy::Lossy)
        );
    }

    #[test]
    fn corrupt_count_entry_is_rebuilt_as_a_cache_miss() {
        let directory = tempdir().unwrap();
        let cache = Cache {
            root: directory.path().to_path_buf(),
        };
        let options = CountOptions::default();
        let content_hash = Cache::content_hash(b"content");
        let key = count_key(&content_hash, &options, InvalidUtf8Policy::Skip);
        let path = cache.count_path(&key);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let mut corrupt = CountEntry::new(
            key,
            CachedCount {
                tokens: 7,
                decoded_lossily: false,
            },
        );
        corrupt.tokens = 99;
        write_json_atomic(&path, &corrupt).unwrap();

        assert_eq!(
            cache.lookup_count(&content_hash, &options, InvalidUtf8Policy::Skip),
            None
        );

        let expected = CachedCount {
            tokens: 7,
            decoded_lossily: false,
        };
        cache.store_count(&content_hash, &options, InvalidUtf8Policy::Skip, expected);
        assert_eq!(
            cache.lookup_count(&content_hash, &options, InvalidUtf8Policy::Skip),
            Some(expected)
        );
    }
}
