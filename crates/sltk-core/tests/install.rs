//! Installs a fixture train from a local HTTP server into a temporary
//! `CARGO_HOME`.
//!
//! The train validator requires https, so the fixture train's URLs point at
//! `https://fixture.test/`, and the test's [`Fetch`] rewrites that origin to
//! the local server's plain-HTTP address. The shipped [`HttpFetch`] has no
//! way to fetch over plain HTTP.
//!
//! [`HttpFetch`]: sltk_core::install::HttpFetch

use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::path::Path;
use std::sync::Arc;

use sltk_core::install::{Fetch, InstallError, bin_dir, install, sha256_hex};
use sltk_core::platform::{Host, Plan, PlanError};
use sltk_core::train::{Platform, Train};

const ORIGIN: &str = "https://fixture.test/";

/// Serves each path in `files` over HTTP/1.0 on a loopback port, and 404 for
/// any other path, until the test process exits. Returns the server's origin.
fn serve(files: BTreeMap<String, Vec<u8>>) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let origin = format!("http://{}/", listener.local_addr().unwrap());
    let files = Arc::new(files);
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut request = String::new();
            if reader.read_line(&mut request).is_err() {
                continue;
            }
            // Read the headers up to the blank line.
            let mut line = String::new();
            while reader.read_line(&mut line).is_ok_and(|n| n > 2) {
                line.clear();
            }
            let path = request.split(' ').nth(1).unwrap_or("/").trim_start_matches('/');
            let response = match files.get(path) {
                Some(body) => {
                    let mut head = format!(
                        "HTTP/1.0 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        body.len()
                    )
                    .into_bytes();
                    head.extend_from_slice(body);
                    head
                }
                None => b"HTTP/1.0 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_vec(),
            };
            let _ = stream.write_all(&response);
        }
    });
    origin
}

/// Fetches from the local server, through no proxy.
struct LocalFetch {
    origin: String,
    agent: ureq::Agent,
}

impl LocalFetch {
    fn new(origin: String) -> LocalFetch {
        let agent = ureq::Agent::config_builder().proxy(None).build().into();
        LocalFetch { origin, agent }
    }
}

impl Fetch for LocalFetch {
    fn open(&self, url: &str) -> Result<Box<dyn Read>, String> {
        let path = url
            .strip_prefix(ORIGIN)
            .ok_or_else(|| format!("{url} is not a fixture URL"))?;
        let response = self
            .agent
            .get(format!("{}{path}", self.origin))
            .call()
            .map_err(|e| e.to_string())?;
        Ok(Box::new(response.into_body().into_reader()))
    }
}

/// A `.tar.xz` laid out as dist lays one out: a top-level directory holding
/// the binary and a README.
fn dist_archive(dir: &str, binary: &str, contents: &[u8]) -> Vec<u8> {
    let mut builder = tar::Builder::new(Vec::new());
    for (name, data, mode) in [("README.md", &b"read me"[..], 0o644), (binary, contents, 0o755)] {
        let mut header = tar::Header::new_gnu();
        header.set_size(data.len() as u64);
        header.set_mode(mode);
        header.set_cksum();
        builder.append_data(&mut header, format!("{dir}/{name}"), data).unwrap();
    }
    let tar = builder.into_inner().unwrap();
    let mut out = Vec::new();
    lzma_rs::xz_compress(&mut &tar[..], &mut out).unwrap();
    out
}

/// One tool's entry in the fixture train, with its archive.
struct Fixture {
    name: &'static str,
    tier: &'static str,
    archive: Vec<u8>,
}

impl Fixture {
    fn new(name: &'static str, tier: &'static str) -> Fixture {
        let contents = format!("#!/bin/sh\necho {name} 9.9.9\n");
        let archive = dist_archive(
            &format!("{name}-cli-x86_64-unknown-linux-gnu"),
            name,
            contents.as_bytes(),
        );
        Fixture { name, tier, archive }
    }

    fn file(&self) -> String {
        format!("{}-cli-x86_64-unknown-linux-gnu.tar.xz", self.name)
    }
}

/// A train of `tools`, each pinned to its archive's sha256 unless `wrong`
/// names it, and the server that serves the archives.
fn train_and_server(tools: &[Fixture], wrong: &str) -> (Train, LocalFetch) {
    let mut text = String::new();
    let mut files = BTreeMap::new();
    for tool in tools {
        let sha256 = if tool.name == wrong { "0".repeat(64) } else { sha256_hex(&tool.archive) };
        text.push_str(&format!(
            r#"
[[tool]]
name = "{name}"
tier = "{tier}"
binary = "{name}"
version = "9.9.9"
url = "{ORIGIN}v{{version}}/{{archive}}"
[tool.platforms.x86_64-unknown-linux-gnu]
archive = "{file}"
sha256 = "{sha256}"
"#,
            name = tool.name,
            tier = tool.tier,
            file = tool.file(),
        ));
        files.insert(format!("v9.9.9/{}", tool.file()), tool.archive.clone());
    }
    (Train::parse(&text).unwrap(), LocalFetch::new(serve(files)))
}

fn tools() -> Vec<Fixture> {
    vec![
        Fixture::new("fab", "essential"),
        Fixture::new("arcl", "essential"),
        Fixture::new("mire", "optional"),
    ]
}

fn linux() -> Host {
    Host::Supported(Platform::LinuxX86_64)
}

/// The names and contents of the files in `dir`, sorted by name.
fn snapshot(dir: &Path) -> Vec<(String, Vec<u8>)> {
    let mut files: Vec<_> = std::fs::read_dir(dir)
        .unwrap()
        .map(|entry| {
            let entry = entry.unwrap();
            (
                entry.file_name().into_string().unwrap(),
                std::fs::read(entry.path()).unwrap(),
            )
        })
        .collect();
    files.sort();
    files
}

#[test]
fn a_good_hash_installs_the_essentials_into_cargo_home_bin() {
    let (train, fetch) = train_and_server(&tools(), "");
    let cargo_home = tempfile::tempdir().unwrap();
    let bin = bin_dir(Some(cargo_home.path().into()), Some("/nonexistent".into())).unwrap();
    let plan = Plan::new(&train, linux(), &[]).unwrap();
    let installed = install(&plan, &fetch, &bin).unwrap();
    let names: Vec<_> = installed.iter().map(|tool| tool.name.as_str()).collect();
    assert_eq!(names, ["fab", "arcl"]);
    let files = snapshot(&bin);
    assert_eq!(files.len(), 2);
    assert_eq!(files[1], ("fab".to_owned(), b"#!/bin/sh\necho fab 9.9.9\n".to_vec()));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(bin.join("arcl")).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o755);
    }
}

#[test]
fn with_mire_installs_mire_too() {
    let (train, fetch) = train_and_server(&tools(), "");
    let cargo_home = tempfile::tempdir().unwrap();
    let bin = cargo_home.path().join("bin");
    let plan = Plan::new(&train, linux(), &["mire"]).unwrap();
    install(&plan, &fetch, &bin).unwrap();
    let names: Vec<_> = snapshot(&bin).into_iter().map(|(name, _)| name).collect();
    assert_eq!(names, ["arcl", "fab", "mire"]);
}

#[test]
fn a_bad_hash_writes_nothing() {
    let (train, fetch) = train_and_server(&tools(), "arcl");
    let cargo_home = tempfile::tempdir().unwrap();
    let bin = cargo_home.path().join("bin");
    std::fs::create_dir(&bin).unwrap();
    std::fs::write(bin.join("fab"), "old fab").unwrap();
    let before = snapshot(&bin);
    let plan = Plan::new(&train, linux(), &[]).unwrap();
    let err = install(&plan, &fetch, &bin).unwrap_err();
    assert!(
        matches!(&err, InstallError::HashMismatch { tool, platform, .. }
            if tool == "arcl" && platform == "x86_64-unknown-linux-gnu"),
        "{err:?}"
    );
    assert_eq!(snapshot(&bin), before);
}

#[test]
fn a_missing_archive_writes_nothing() {
    let (train, _) = train_and_server(&tools(), "");
    let fetch = LocalFetch::new(serve(BTreeMap::new()));
    let cargo_home = tempfile::tempdir().unwrap();
    let bin = cargo_home.path().join("bin");
    let plan = Plan::new(&train, linux(), &[]).unwrap();
    let err = install(&plan, &fetch, &bin).unwrap_err();
    assert!(
        matches!(&err, InstallError::Download { tool, .. } if tool == "fab"),
        "{err:?}"
    );
    assert!(!bin.exists());
}

#[test]
fn with_names_checked_against_the_train() {
    let (train, _) = train_and_server(&tools(), "");
    assert_eq!(
        Plan::new(&train, linux(), &["fab"]).unwrap_err(),
        PlanError::EssentialTool { tool: "fab".to_owned() }
    );
    assert_eq!(
        Plan::new(&train, linux(), &["dalil"]).unwrap_err(),
        PlanError::UnknownTool { tool: "dalil".to_owned() }
    );
}
