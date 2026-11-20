#![cfg(target_os = "linux")]

use std::io::ErrorKind;
use std::net::TcpListener;
use std::process::{Child, Command, ExitStatus};
use std::thread;
use std::time::{Duration, Instant};

struct OwnedChild(Child);
impl Drop for OwnedChild {
    fn drop(&mut self) {
        // SAFETY: the test child is spawned in its own process group.
        unsafe {
            libc::kill(-(self.0.id() as i32), libc::SIGKILL);
        }
        let _ = self.0.wait();
    }
}

fn finish(child: &mut Child) -> ExitStatus {
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            return status;
        }
        assert!(
            Instant::now() < deadline,
            "CLI did not terminate after SIGINT"
        );
        thread::sleep(Duration::from_millis(10));
    }
}

fn matching_processes(needle: &str) -> Vec<i32> {
    std::fs::read_dir("/proc")
        .unwrap()
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let pid = entry.file_name().to_string_lossy().parse::<i32>().ok()?;
            let bytes = std::fs::read(entry.path().join("cmdline")).ok()?;
            bytes
                .windows(needle.len())
                .any(|part| part == needle.as_bytes())
                .then_some(pid)
        })
        .collect()
}

#[test]
fn sigint_during_history_transport_returns_130_and_leaves_no_git_helpers() {
    use std::os::unix::process::CommandExt;
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let url = format!("http://{}/history.git", listener.local_addr().unwrap());
    let mut command = Command::new(assert_cmd::cargo::cargo_bin("greenshift"));
    command
        .process_group(0)
        .env_remove("GREENSHIFT_HISTORY_TOKEN")
        .env("http_proxy", "")
        .env("HTTP_PROXY", "")
        .env("ALL_PROXY", "")
        .env("NO_PROXY", "127.0.0.1")
        .args(["history", "cache-key", "--remote", &url]);
    let mut child = OwnedChild(command.spawn().unwrap());
    let deadline = Instant::now() + Duration::from_secs(5);
    let stream = loop {
        match listener.accept() {
            Ok((stream, _)) => break stream,
            Err(error) if error.kind() == ErrorKind::WouldBlock => {
                assert!(Instant::now() < deadline, "git never connected");
                assert!(
                    child.0.try_wait().unwrap().is_none(),
                    "CLI exited before transport"
                );
                thread::sleep(Duration::from_millis(10));
            }
            Err(error) => panic!("accept: {error}"),
        }
    };
    // A live peer never returns headers; the transport must be cancelled rather
    // than allowed to run to its ordinary 60-second timeout.
    assert!(matching_processes(&url).len() >= 2);
    // SAFETY: pid is our still-running child, not the caller's process group.
    assert_eq!(unsafe { libc::kill(child.0.id() as i32, libc::SIGINT) }, 0);
    let status = finish(&mut child.0);
    let deadline = Instant::now() + Duration::from_millis(500);
    while !matching_processes(&url).is_empty() && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(10));
    }
    let remaining = matching_processes(&url);
    // Clean even a deliberately regressed implementation before asserting.
    for pid in &remaining {
        // SAFETY: only descendants observed with this unique loopback URL.
        unsafe {
            libc::kill(*pid, libc::SIGKILL);
        }
    }
    drop(stream);
    assert_eq!(status.code(), Some(130));
    assert!(
        remaining.is_empty(),
        "transport processes survived SIGINT: {remaining:?}"
    );
}

#[test]
fn sigint_outside_transport_does_not_disappear_during_backend_listing() {
    use std::os::unix::{fs::PermissionsExt, process::CommandExt};
    let dir = tempfile::tempdir().unwrap();
    let marker = dir.path().join("started");
    let repo = dir.path().join("remote.git");
    assert!(Command::new("git")
        .args(["init", "--bare"])
        .arg(&repo)
        .output()
        .unwrap()
        .status
        .success());
    let canonical = dir.path().join("canonical.sqlite");
    drop(greenshift::store::Store::open(&canonical).unwrap());
    greenshift::history::Remote::new(repo.to_str().unwrap())
        .unwrap()
        .publish(&canonical, None)
        .unwrap();
    let restored = dir.path().join("restored.sqlite");
    let cargo = dir.path().join("cargo");
    std::fs::write(
        &cargo,
        "#!/bin/sh\nprintf started > \"$LIST_MARKER\"\nsleep 15\n",
    )
    .unwrap();
    std::fs::set_permissions(&cargo, std::fs::Permissions::from_mode(0o700)).unwrap();
    let mut command = Command::new(assert_cmd::cargo::cargo_bin("greenshift"));
    command
        .process_group(0)
        .env(
            "PATH",
            format!(
                "{}:{}",
                dir.path().display(),
                std::env::var("PATH").unwrap()
            ),
        )
        .env("LIST_MARKER", &marker)
        .args(["run", "nextest", "--history-remote"])
        .arg(&repo)
        .arg("--db")
        .arg(&restored);
    let mut child = OwnedChild(command.spawn().unwrap());
    let deadline = Instant::now() + Duration::from_secs(3);
    while !marker.exists() {
        assert!(Instant::now() < deadline, "backend listing did not begin");
        thread::sleep(Duration::from_millis(10));
    }
    assert!(
        restored.exists(),
        "transport must finish before backend signal test"
    );
    // SAFETY: signal our own child; its backend must not cause ignored SIGINT.
    assert_eq!(unsafe { libc::kill(child.0.id() as i32, libc::SIGINT) }, 0);
    assert_eq!(finish(&mut child.0).code(), Some(130));
}
