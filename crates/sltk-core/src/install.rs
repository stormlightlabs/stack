//! The install itself: download each archive a plan names, check it against
//! the train's sha256, take the binary out of it and put it in the bin
//! directory.
//!
//! Nothing touches the disk until every archive has been downloaded, checked
//! and unpacked in memory. A download that fails or does not match its hash
//! writes nothing at all.

use std::ffi::OsString;
use std::fmt::Write as _;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use sha2::{Digest, Sha256};
use tar::EntryType;

use crate::platform::{Plan, PlannedTool};

/// The most bytes `sltk` downloads for one archive. dist's archives of fab
/// and arcl are under 10 MB.
pub const MAX_DOWNLOAD: u64 = 64 << 20;

/// The most bytes one archive may unpack to, counting every file in it.
pub const MAX_UNPACKED: u64 = 256 << 20;

/// The most bytes one binary may hold. arcl 0.3.0 is about 17 MB.
pub const MAX_BINARY: u64 = 128 << 20;

/// Why an install stopped. When it stops before the renames, which is every
/// case but [`InstallError::Io`] on a rename, the bin directory is as it was.
#[derive(Debug, thiserror::Error)]
pub enum InstallError {
    /// The plan has tools with no archive for the host.
    #[error("sltk: the plan has tools with no archive for {host}")]
    Incomplete {
        /// The host, as the plan shows it.
        host: String,
    },
    /// The download failed.
    #[error("sltk: could not download {tool} from {url}: {message}")]
    Download {
        /// The tool's name.
        tool: String,
        /// The URL it was downloaded from.
        url: String,
        /// What the HTTP client reported.
        message: String,
    },
    /// The archive, its unpacked contents or its binary passed a size cap.
    #[error("sltk: the {what} of {tool} is larger than {limit} bytes")]
    TooLarge {
        /// The tool's name.
        tool: String,
        /// `archive`, `unpacked archive` or `binary`.
        what: &'static str,
        /// The cap, in bytes.
        limit: u64,
    },
    /// The archive's sha256 is not the one the train pins.
    #[error(
        "sltk: the archive of {tool} for {platform} does not match the train's sha256\n  expected {expected}\n  actual   {actual}"
    )]
    HashMismatch {
        /// The tool's name.
        tool: String,
        /// The platform the archive is for.
        platform: String,
        /// The sha256 the train pins.
        expected: String,
        /// The sha256 of the downloaded bytes.
        actual: String,
    },
    /// The archive is not a readable `.tar.xz`.
    #[error("sltk: the archive of {tool} is not a readable .tar.xz: {message}")]
    BadArchive {
        /// The tool's name.
        tool: String,
        /// What the xz or tar reader reported.
        message: String,
    },
    /// The archive holds an entry `sltk` refuses to unpack from.
    #[error("sltk: the archive of {tool} holds {entry:?}, {reason}")]
    UnsafeEntry {
        /// The tool's name.
        tool: String,
        /// The entry's path, as the archive gives it.
        entry: String,
        /// Why the entry is refused.
        reason: &'static str,
    },
    /// The archive holds no regular file named for the tool's binary.
    #[error("sltk: the archive of {tool} holds no file named {binary}")]
    NoBinary {
        /// The tool's name.
        tool: String,
        /// The binary the train names.
        binary: String,
    },
    /// The archive holds more than one regular file named for the binary.
    #[error("sltk: the archive of {tool} holds more than one file named {binary}")]
    ManyBinaries {
        /// The tool's name.
        tool: String,
        /// The binary the train names.
        binary: String,
    },
    /// Neither `CARGO_HOME` nor `HOME` is set.
    #[error("sltk: neither CARGO_HOME nor HOME is set, so there is no bin directory to install into")]
    NoBinDir,
    /// Writing to the bin directory failed.
    #[error("sltk: {}: {source}", path.display())]
    Io {
        /// The file or directory that failed.
        path: PathBuf,
        /// The error the OS reported.
        source: io::Error,
    },
}

/// A tool `sltk install` put in the bin directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Installed {
    /// The tool's name.
    pub name: String,
    /// The release installed.
    pub version: String,
    /// Where the binary now is.
    pub path: PathBuf,
}

/// Downloads over HTTPS with a blocking client.
///
/// It refuses plain HTTP, redirects included, and trusts the platform's
/// certificate store. It reads `HTTPS_PROXY`, `ALL_PROXY` and `NO_PROXY`
/// from the environment, and on Linux `SSL_CERT_FILE` and `SSL_CERT_DIR`.
pub struct HttpFetch {
    agent: ureq::Agent,
}

impl HttpFetch {
    /// A client with a 30-second connect timeout and a 10-minute cap on each
    /// download.
    pub fn new() -> HttpFetch {
        let tls = ureq::tls::TlsConfig::builder()
            .root_certs(ureq::tls::RootCerts::PlatformVerifier)
            .build();
        let agent = ureq::Agent::config_builder()
            .https_only(true)
            .tls_config(tls)
            .timeout_connect(Some(Duration::from_secs(30)))
            .timeout_global(Some(Duration::from_secs(600)))
            .build()
            .into();
        HttpFetch { agent }
    }
}

impl Default for HttpFetch {
    fn default() -> HttpFetch {
        HttpFetch::new()
    }
}

impl Fetch for HttpFetch {
    fn open(&self, url: &str) -> Result<Box<dyn Read>, String> {
        let response = self.agent.get(url).call().map_err(|err| err.to_string())?;
        Ok(Box::new(response.into_body().into_reader()))
    }
}

/// The caps one install works under; tests shrink them.
#[derive(Debug, Clone, Copy)]
struct Limits {
    download: u64,
    unpacked: u64,
    binary: u64,
}

impl Limits {
    /// The caps `install` uses.
    const DEFAULT: Limits = Limits { download: MAX_DOWNLOAD, unpacked: MAX_UNPACKED, binary: MAX_BINARY };
}

/// A `Vec` that refuses to grow past a cap, so a small archive cannot unpack
/// into unbounded memory.
struct CappedWriter {
    bytes: Vec<u8>,
    limit: u64,
    overflowed: bool,
}

impl Write for CappedWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if self.bytes.len() as u64 + buf.len() as u64 > self.limit {
            self.overflowed = true;
            return Err(io::Error::other("unpacked size cap reached"));
        }
        self.bytes.extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// Opens a URL for reading. [`HttpFetch`] is the real one; tests supply
/// their own to serve archives from a local server.
pub trait Fetch {
    /// Opens `url` and returns its body, or what went wrong.
    fn open(&self, url: &str) -> Result<Box<dyn Read>, String>;
}

/// The directory tools install into: `$CARGO_HOME/bin` when `CARGO_HOME` is
/// set and not empty, else `$HOME/.cargo/bin`. This is where dist's own
/// installers put them.
pub fn bin_dir(cargo_home: Option<OsString>, home: Option<OsString>) -> Result<PathBuf, InstallError> {
    match (cargo_home.filter(|v| !v.is_empty()), home.filter(|v| !v.is_empty())) {
        (Some(cargo_home), _) => Ok(PathBuf::from(cargo_home).join("bin")),
        (None, Some(home)) => Ok(PathBuf::from(home).join(".cargo").join("bin")),
        (None, None) => Err(InstallError::NoBinDir),
    }
}

/// Installs every tool in `plan` into `bin`, creating `bin` when it is
/// missing.
///
/// It downloads every archive with `fetch`, checks each against the
/// train's sha256 and takes each binary out in memory before it writes
/// anything. Then it writes each binary to a temporary file in `bin` with
/// mode 0755, and renames each over its target. Any failure before the
/// renames leaves `bin` as it was, and removes the temporary files. A
/// rename that fails leaves the tools before it installed and the rest as
/// they were.
pub fn install(plan: &Plan, fetch: &dyn Fetch, bin: &Path) -> Result<Vec<Installed>, InstallError> {
    install_with(plan, fetch, bin, Limits::DEFAULT)
}

/// The lowercase hexadecimal sha256 of `bytes`.
pub fn sha256_hex(bytes: &[u8]) -> String {
    let mut hex = String::with_capacity(64);
    for byte in Sha256::digest(bytes) {
        let _ = write!(hex, "{byte:02x}");
    }
    hex
}

/// [`install`] under the caps in `limits`.
fn install_with(plan: &Plan, fetch: &dyn Fetch, bin: &Path, limits: Limits) -> Result<Vec<Installed>, InstallError> {
    let Some(platform) = plan.host.platform().filter(|_| plan.is_complete()) else {
        return Err(InstallError::Incomplete { host: plan.host.to_string() });
    };
    let mut binaries = Vec::with_capacity(plan.install.len());
    for tool in &plan.install {
        let archive = download(tool, fetch, limits.download)?;
        verify(tool, platform.as_str(), &archive)?;
        binaries.push(extract(&tool.name, &tool.binary, &archive, limits)?);
    }
    std::fs::create_dir_all(bin).map_err(|source| InstallError::Io { path: bin.to_owned(), source })?;
    let mut staged: Vec<PathBuf> = Vec::with_capacity(binaries.len());
    for (tool, bytes) in plan.install.iter().zip(&binaries) {
        match stage(bin, &tool.binary, bytes) {
            Ok(temp) => staged.push(temp),
            Err(err) => {
                remove_all(&staged);
                return Err(err);
            }
        }
    }
    let mut installed = Vec::with_capacity(staged.len());
    for (i, (tool, temp)) in plan.install.iter().zip(&staged).enumerate() {
        let path = bin.join(&tool.binary);
        if let Err(source) = std::fs::rename(temp, &path) {
            remove_all(&staged[i..]);
            return Err(InstallError::Io { path, source });
        }
        installed.push(Installed { name: tool.name.clone(), version: tool.version.clone(), path });
    }
    Ok(installed)
}

/// Downloads a tool's archive into memory, refusing more than `limit` bytes.
fn download(tool: &PlannedTool, fetch: &dyn Fetch, limit: u64) -> Result<Vec<u8>, InstallError> {
    let download_err =
        |message: String| InstallError::Download { tool: tool.name.clone(), url: tool.url.clone(), message };
    let reader = fetch.open(&tool.url).map_err(download_err)?;
    match read_capped(reader, limit) {
        Ok(Some(bytes)) => Ok(bytes),
        Ok(None) => Err(InstallError::TooLarge { tool: tool.name.clone(), what: "archive", limit }),
        Err(err) => Err(download_err(err.to_string())),
    }
}

/// Reads all of `reader`, or `None` when it holds more than `limit` bytes.
fn read_capped(reader: impl Read, limit: u64) -> io::Result<Option<Vec<u8>>> {
    let mut bytes = Vec::new();
    reader.take(limit.saturating_add(1)).read_to_end(&mut bytes)?;
    Ok((bytes.len() as u64 <= limit).then_some(bytes))
}

/// Checks the downloaded bytes against the sha256 the train pins.
fn verify(tool: &PlannedTool, platform: &str, archive: &[u8]) -> Result<(), InstallError> {
    let actual = sha256_hex(archive);
    if actual == tool.archive.sha256 {
        return Ok(());
    }
    Err(InstallError::HashMismatch {
        tool: tool.name.clone(),
        platform: platform.to_owned(),
        expected: tool.archive.sha256.clone(),
        actual,
    })
}

/// Takes the one regular file named `binary` out of a `.tar.xz`.
///
/// It reads every entry and refuses the whole archive when any entry has an
/// absolute path, a `..` component, or is a symlink, a hard link or a
/// device. dist's archives hold none of those.
fn extract(tool: &str, binary: &str, archive: &[u8], limits: Limits) -> Result<Vec<u8>, InstallError> {
    let bad = |message: String| InstallError::BadArchive { tool: tool.to_owned(), message };
    let mut tar = CappedWriter { bytes: Vec::new(), limit: limits.unpacked, overflowed: false };
    if let Err(err) = lzma_rs::xz_decompress(&mut &archive[..], &mut tar) {
        if tar.overflowed {
            return Err(InstallError::TooLarge {
                tool: tool.to_owned(),
                what: "unpacked archive",
                limit: limits.unpacked,
            });
        }
        return Err(bad(err.to_string()));
    }
    let mut reader = tar::Archive::new(&tar.bytes[..]);
    let mut found = None;
    for entry in reader.entries().map_err(|err| bad(err.to_string()))? {
        let mut entry = entry.map_err(|err| bad(err.to_string()))?;
        let path = String::from_utf8_lossy(&entry.path_bytes()).into_owned();
        let unsafe_entry = |reason| InstallError::UnsafeEntry { tool: tool.to_owned(), entry: path.clone(), reason };
        if path.starts_with('/') {
            return Err(unsafe_entry("an absolute path"));
        }
        if path.split('/').any(|part| part == "..") {
            return Err(unsafe_entry("a path that leaves the archive"));
        }
        match entry.header().entry_type() {
            EntryType::Regular | EntryType::Continuous => {}
            EntryType::Directory | EntryType::XGlobalHeader | EntryType::XHeader => continue,
            EntryType::Symlink => return Err(unsafe_entry("a symlink")),
            EntryType::Link => return Err(unsafe_entry("a hard link")),
            _ => return Err(unsafe_entry("an entry that is not a file or directory")),
        }
        if path.rsplit('/').next() != Some(binary) {
            continue;
        }
        if found.is_some() {
            return Err(InstallError::ManyBinaries { tool: tool.to_owned(), binary: binary.to_owned() });
        }
        let too_large = || InstallError::TooLarge { tool: tool.to_owned(), what: "binary", limit: limits.binary };
        if entry.header().size().map_err(|err| bad(err.to_string()))? > limits.binary {
            return Err(too_large());
        }
        match read_capped(&mut entry, limits.binary) {
            Ok(Some(bytes)) => found = Some(bytes),
            Ok(None) => return Err(too_large()),
            Err(err) => return Err(bad(err.to_string())),
        }
    }
    found.ok_or_else(|| InstallError::NoBinary { tool: tool.to_owned(), binary: binary.to_owned() })
}

/// Writes `bytes` to a new temporary file in `bin`, with mode 0755 on Unix,
/// and syncs it. Returns the file's path; on failure the file is removed.
fn stage(bin: &Path, binary: &str, bytes: &[u8]) -> Result<PathBuf, InstallError> {
    let temp = bin.join(format!(".{binary}.sltk-{}.tmp", std::process::id()));
    let result = write_executable(&temp, bytes);
    if let Err(source) = result {
        let _ = std::fs::remove_file(&temp);
        return Err(InstallError::Io { path: temp, source });
    }
    Ok(temp)
}

/// Creates `path`, which must not exist, writes `bytes`, makes it
/// executable and syncs it.
fn write_executable(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
        options.mode(0o755);
        let mut file = options.open(path)?;
        file.write_all(bytes)?;
        // The mode given at creation is reduced by the umask.
        file.set_permissions(std::fs::Permissions::from_mode(0o755))?;
        file.sync_all()
    }
    #[cfg(not(unix))]
    {
        let mut file = options.open(path)?;
        file.write_all(bytes)?;
        file.sync_all()
    }
}

/// Removes each file, ignoring failures.
fn remove_all(paths: &[PathBuf]) {
    for path in paths {
        let _ = std::fs::remove_file(path);
    }
}

#[cfg(test)]
mod tests {
    use std::io::Read;
    use std::path::Path;

    use super::{Fetch, InstallError, Limits, bin_dir, extract, install_with, read_capped, sha256_hex};
    use crate::platform::{Host, Plan};
    use crate::train::{Platform, Train};

    const SMALL: Limits = Limits { download: 1 << 20, unpacked: 1 << 20, binary: 1 << 16 };

    /// One tar entry: its raw path, type and contents.
    struct Entry<'a> {
        path: &'a str,
        kind: tar::EntryType,
        data: &'a [u8],
        link: Option<&'a str>,
    }

    fn file<'a>(path: &'a str, data: &'a [u8]) -> Entry<'a> {
        Entry { path, kind: tar::EntryType::Regular, data, link: None }
    }

    /// A `.tar.xz` of `entries`. Paths go into the header as written, so a
    /// test can build the `..` and absolute paths `tar::Builder` refuses.
    fn tar_xz(entries: &[Entry]) -> Vec<u8> {
        let mut builder = tar::Builder::new(Vec::new());
        for entry in entries {
            let mut header = tar::Header::new_gnu();
            let name = &mut header.as_old_mut().name;
            name[..entry.path.len()].copy_from_slice(entry.path.as_bytes());
            header.set_entry_type(entry.kind);
            header.set_size(entry.data.len() as u64);
            header.set_mode(0o755);
            if let Some(link) = entry.link {
                header.set_link_name(link).unwrap();
            }
            header.set_cksum();
            builder.append(&header, entry.data).unwrap();
        }
        let tar = builder.into_inner().unwrap();
        let mut out = Vec::new();
        lzma_rs::xz_compress(&mut &tar[..], &mut out).unwrap();
        out
    }

    fn extract_err(entries: &[Entry]) -> InstallError {
        extract("alpha", "alpha", &tar_xz(entries), SMALL).unwrap_err()
    }

    #[test]
    fn the_hash_is_lowercase_hex() {
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn the_binary_comes_out_of_its_top_level_directory() {
        let archive = tar_xz(&[
            Entry { path: "alpha-cli/", kind: tar::EntryType::Directory, data: b"", link: None },
            file("alpha-cli/README.md", b"read me"),
            file("alpha-cli/alpha", b"the binary"),
        ]);
        assert_eq!(extract("alpha", "alpha", &archive, SMALL).unwrap(), b"the binary");
    }

    #[test]
    fn an_archive_without_the_binary_is_refused() {
        let err = extract_err(&[file("alpha-cli/alphabet", b"x"), file("alpha-cli/README.md", b"x")]);
        assert_eq!(err.to_string(), "sltk: the archive of alpha holds no file named alpha");
    }

    #[test]
    fn two_files_named_for_the_binary_are_refused() {
        let err = extract_err(&[file("a/alpha", b"x"), file("b/alpha", b"y")]);
        assert!(matches!(err, InstallError::ManyBinaries { .. }), "{err:?}");
    }

    #[test]
    fn a_path_with_dot_dot_is_refused() {
        let err = extract_err(&[file("alpha-cli/../../alpha", b"x")]);
        assert_eq!(
            err.to_string(),
            "sltk: the archive of alpha holds \"alpha-cli/../../alpha\", a path that leaves the archive"
        );
    }

    #[test]
    fn a_dot_dot_entry_elsewhere_in_the_archive_is_refused() {
        let err = extract_err(&[file("alpha-cli/alpha", b"x"), file("../.bashrc", b"y")]);
        assert!(
            matches!(
                err,
                InstallError::UnsafeEntry { reason: "a path that leaves the archive", .. }
            ),
            "{err:?}"
        );
    }

    #[test]
    fn an_absolute_path_is_refused() {
        let err = extract_err(&[file("/usr/local/bin/alpha", b"x")]);
        assert!(
            matches!(err, InstallError::UnsafeEntry { reason: "an absolute path", .. }),
            "{err:?}"
        );
    }

    #[test]
    fn a_symlink_is_refused() {
        let err = extract_err(&[Entry {
            path: "alpha-cli/alpha",
            kind: tar::EntryType::Symlink,
            data: b"",
            link: Some("/bin/sh"),
        }]);
        assert!(
            matches!(err, InstallError::UnsafeEntry { reason: "a symlink", .. }),
            "{err:?}"
        );
    }

    #[test]
    fn a_hard_link_is_refused() {
        let err = extract_err(&[
            file("alpha-cli/alpha", b"x"),
            Entry { path: "alpha-cli/other", kind: tar::EntryType::Link, data: b"", link: Some("alpha-cli/alpha") },
        ]);
        assert!(
            matches!(err, InstallError::UnsafeEntry { reason: "a hard link", .. }),
            "{err:?}"
        );
    }

    #[test]
    fn a_device_is_refused() {
        let err = extract_err(&[Entry { path: "alpha-cli/alpha", kind: tar::EntryType::Char, data: b"", link: None }]);
        assert!(matches!(err, InstallError::UnsafeEntry { .. }), "{err:?}");
    }

    #[test]
    fn an_oversize_binary_is_refused() {
        let big = vec![0u8; (SMALL.binary + 1) as usize];
        let err = extract_err(&[file("alpha-cli/alpha", &big)]);
        assert_eq!(err.to_string(), "sltk: the binary of alpha is larger than 65536 bytes");
    }

    #[test]
    fn an_archive_that_unpacks_past_the_cap_is_refused() {
        let big = vec![0u8; (SMALL.unpacked + 1) as usize];
        let err = extract_err(&[file("alpha-cli/README.md", &big)]);
        assert!(
            matches!(err, InstallError::TooLarge { what: "unpacked archive", .. }),
            "{err:?}"
        );
    }

    #[test]
    fn bytes_that_are_not_xz_are_refused() {
        let err = extract("alpha", "alpha", b"not an archive", SMALL).unwrap_err();
        assert!(matches!(err, InstallError::BadArchive { .. }), "{err:?}");
    }

    #[test]
    fn a_read_past_the_cap_returns_none() {
        assert_eq!(read_capped(&b"abcd"[..], 4).unwrap(), Some(b"abcd".to_vec()));
        assert_eq!(read_capped(&b"abcde"[..], 4).unwrap(), None);
    }

    #[test]
    fn the_bin_directory_follows_cargo_home_then_home() {
        let some = |s: &str| Some(s.into());
        assert_eq!(bin_dir(some("/c"), some("/h")).unwrap(), Path::new("/c/bin"));
        assert_eq!(bin_dir(some(""), some("/h")).unwrap(), Path::new("/h/.cargo/bin"));
        assert_eq!(bin_dir(None, some("/h")).unwrap(), Path::new("/h/.cargo/bin"));
        assert!(matches!(bin_dir(None, None), Err(InstallError::NoBinDir)));
    }

    /// Serves fixed bodies by URL.
    struct MapFetch(Vec<(String, Vec<u8>)>);

    impl Fetch for MapFetch {
        fn open(&self, url: &str) -> Result<Box<dyn Read>, String> {
            match self.0.iter().find(|(u, _)| u == url) {
                Some((_, body)) => Ok(Box::new(std::io::Cursor::new(body.clone()))),
                None => Err(format!("no fixture for {url}")),
            }
        }
    }

    /// A train of `alpha` and `beta` with the hashes given, and the fetcher
    /// that serves their archives.
    fn fixture(alpha_hash: &str, beta_hash: &str) -> (Train, MapFetch, Vec<u8>, Vec<u8>) {
        let alpha = tar_xz(&[file("alpha-cli/alpha", b"alpha 2")]);
        let beta = tar_xz(&[file("beta-cli/beta", b"beta 2")]);
        let hash = |given: &str, bytes: &[u8]| if given.is_empty() { sha256_hex(bytes) } else { given.to_owned() };
        let text = format!(
            r#"
[[tool]]
name = "alpha"
tier = "essential"
binary = "alpha"
version = "2.0.0"
url = "https://fixture.test/{{version}}/{{archive}}"
[tool.platforms.x86_64-unknown-linux-gnu]
archive = "alpha-x86_64-unknown-linux-gnu.tar.xz"
sha256 = "{}"

[[tool]]
name = "beta"
tier = "essential"
binary = "beta"
version = "2.0.0"
url = "https://fixture.test/{{version}}/{{archive}}"
[tool.platforms.x86_64-unknown-linux-gnu]
archive = "beta-x86_64-unknown-linux-gnu.tar.xz"
sha256 = "{}"
"#,
            hash(alpha_hash, &alpha),
            hash(beta_hash, &beta),
        );
        let fetch = MapFetch(vec![
            (
                "https://fixture.test/2.0.0/alpha-x86_64-unknown-linux-gnu.tar.xz".into(),
                alpha.clone(),
            ),
            (
                "https://fixture.test/2.0.0/beta-x86_64-unknown-linux-gnu.tar.xz".into(),
                beta.clone(),
            ),
        ]);
        (Train::parse(&text).unwrap(), fetch, alpha, beta)
    }

    fn linux_plan(train: &Train) -> Plan {
        Plan::new(train, Host::Supported(Platform::LinuxX86_64), &[]).unwrap()
    }

    fn entries(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(dir)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().into_string().unwrap())
            .collect();
        names.sort();
        names
    }

    #[test]
    fn an_install_replaces_the_old_binaries_with_executables() {
        let (train, fetch, _, _) = fixture("", "");
        let bin = tempfile::tempdir().unwrap();
        std::fs::write(bin.path().join("alpha"), "alpha 1").unwrap();
        let installed = install_with(&linux_plan(&train), &fetch, bin.path(), SMALL).unwrap();
        assert_eq!(installed.len(), 2);
        assert_eq!(installed[0].path, bin.path().join("alpha"));
        assert_eq!(entries(bin.path()), ["alpha", "beta"]);
        assert_eq!(std::fs::read(bin.path().join("alpha")).unwrap(), b"alpha 2");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(bin.path().join("beta")).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o755);
        }
    }

    #[test]
    fn a_missing_bin_directory_is_created() {
        let (train, fetch, _, _) = fixture("", "");
        let home = tempfile::tempdir().unwrap();
        let bin = home.path().join("cargo").join("bin");
        install_with(&linux_plan(&train), &fetch, &bin, SMALL).unwrap();
        assert_eq!(entries(&bin), ["alpha", "beta"]);
    }

    #[test]
    fn a_bad_hash_on_the_second_tool_installs_neither() {
        let wrong = "0".repeat(64);
        let (train, fetch, _, beta) = fixture("", &wrong);
        let home = tempfile::tempdir().unwrap();
        let bin = home.path().join("bin");
        let err = install_with(&linux_plan(&train), &fetch, &bin, SMALL).unwrap_err();
        assert_eq!(
            err.to_string(),
            format!(
                "sltk: the archive of beta for x86_64-unknown-linux-gnu does not match the train's sha256\n  \
                 expected {wrong}\n  actual   {}",
                sha256_hex(&beta)
            )
        );
        assert!(!bin.exists(), "the bin directory was created");
    }

    #[test]
    fn a_failed_rename_keeps_the_old_binary_and_leaves_no_temporary_file() {
        let (train, fetch, _, _) = fixture("", "");
        let bin = tempfile::tempdir().unwrap();
        std::fs::write(bin.path().join("alpha"), "alpha 1").unwrap();
        // A non-empty directory where beta goes makes its rename fail.
        std::fs::create_dir_all(bin.path().join("beta").join("keep")).unwrap();
        let err = install_with(&linux_plan(&train), &fetch, bin.path(), SMALL).unwrap_err();
        assert!(matches!(err, InstallError::Io { .. }), "{err:?}");
        assert_eq!(entries(bin.path()), ["alpha", "beta"]);
        assert!(bin.path().join("beta").join("keep").is_dir());
    }

    #[test]
    fn a_failed_stage_writes_nothing() {
        let (train, fetch, _, _) = fixture("", "");
        let bin = tempfile::tempdir().unwrap();
        std::fs::write(bin.path().join("alpha"), "alpha 1").unwrap();
        // A file where beta's temporary file goes makes its stage fail.
        let blocker = format!(".beta.sltk-{}.tmp", std::process::id());
        std::fs::create_dir(bin.path().join(&blocker)).unwrap();
        let err = install_with(&linux_plan(&train), &fetch, bin.path(), SMALL).unwrap_err();
        assert!(matches!(err, InstallError::Io { .. }), "{err:?}");
        assert_eq!(entries(bin.path()), [blocker.as_str(), "alpha"]);
        assert_eq!(std::fs::read(bin.path().join("alpha")).unwrap(), b"alpha 1");
    }

    #[test]
    fn an_oversize_download_is_refused() {
        let (train, _, alpha, _) = fixture("", "");
        let mut big = alpha.clone();
        big.resize((SMALL.download + 1) as usize, 0);
        let fetch = MapFetch(vec![(
            "https://fixture.test/2.0.0/alpha-x86_64-unknown-linux-gnu.tar.xz".into(),
            big,
        )]);
        let bin = tempfile::tempdir().unwrap();
        let err = install_with(&linux_plan(&train), &fetch, bin.path(), SMALL).unwrap_err();
        assert_eq!(
            err.to_string(),
            "sltk: the archive of alpha is larger than 1048576 bytes"
        );
    }

    #[test]
    fn a_failed_download_names_the_tool_and_url() {
        let (train, _, _, _) = fixture("", "");
        let bin = tempfile::tempdir().unwrap();
        let err = install_with(&linux_plan(&train), &MapFetch(vec![]), bin.path(), SMALL).unwrap_err();
        assert_eq!(
            err.to_string(),
            "sltk: could not download alpha from https://fixture.test/2.0.0/alpha-x86_64-unknown-linux-gnu.tar.xz: \
             no fixture for https://fixture.test/2.0.0/alpha-x86_64-unknown-linux-gnu.tar.xz"
        );
        assert!(entries(bin.path()).is_empty());
    }

    #[test]
    fn an_incomplete_plan_installs_nothing() {
        let (train, fetch, _, _) = fixture("", "");
        let plan = Plan::new(&train, Host::Supported(Platform::AppleSilicon), &[]).unwrap();
        let bin = tempfile::tempdir().unwrap();
        let err = install_with(&plan, &fetch, bin.path(), SMALL).unwrap_err();
        assert!(matches!(err, InstallError::Incomplete { .. }), "{err:?}");
        assert!(entries(bin.path()).is_empty());
    }
}
