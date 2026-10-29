//! "Connect your Mac" end to end: the real `dove-mesh-host` binary on both
//! sides of the mock edge's DeviceRoom relay. A node (`serve-node`) prints a
//! pairing code, a laptop pairs with it and runs processes there with live
//! stdin/stdout/stderr and the remote exit code; an unpaired device is
//! refused, codes are single use, revoke cuts a device off, and a vanished
//! node reads as a lost connection (255).

#[path = "../../client/tests/support/mock_edge.rs"]
mod mock_edge;

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, Command};

const BIN: &str = env!("CARGO_BIN_EXE_dove-mesh-host");

fn signed_in(dir: &Path) {
    std::fs::create_dir_all(dir).unwrap();
    // Not a JWT: never "expired", so no refresh is attempted.
    let account = serde_json::json!({
        "user_id": "user-1", "email": null, "access_token": "test-token", "refresh_token": ""
    });
    std::fs::write(dir.join("account.json"), account.to_string()).unwrap();
}

fn cmd(edge: &str, dir: &Path) -> Command {
    let mut c = Command::new(BIN);
    c.env("DOVE_EDGE_URL", edge).arg("--data-dir").arg(dir).kill_on_drop(true);
    c
}

struct Out {
    code: i32,
    stdout: Vec<u8>,
    stderr: String,
}

async fn run(edge: &str, dir: &Path, args: &[&str], stdin: &[u8]) -> Out {
    let mut child =
        cmd(edge, dir).args(args).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().unwrap();
    let mut input = child.stdin.take().unwrap();
    let data = stdin.to_vec();
    let feeder = tokio::spawn(async move {
        let _ = input.write_all(&data).await;
        // Dropping `input` closes stdin: the remote must see EOF.
    });
    let out =
        tokio::time::timeout(Duration::from_secs(60), child.wait_with_output()).await.expect("command hung").unwrap();
    feeder.await.unwrap();
    Out {
        code: out.status.code().unwrap_or(-1),
        stdout: out.stdout,
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
    }
}

fn device_id(dir: &Path) -> String {
    let raw = std::fs::read_to_string(dir.join("device.json")).unwrap();
    serde_json::from_str::<serde_json::Value>(&raw).unwrap()["id"].as_str().unwrap().to_owned()
}

/// Starts `serve-node` and returns (child, pairing URI).
async fn serve_node(edge: &str, dir: &Path) -> (Child, String) {
    let mut child = cmd(edge, dir).arg("serve-node").stdout(Stdio::piped()).stderr(Stdio::inherit()).spawn().unwrap();
    let mut lines = BufReader::new(child.stdout.take().unwrap()).lines();
    let uri = tokio::time::timeout(Duration::from_secs(20), async {
        while let Some(line) = lines.next_line().await.unwrap() {
            if let Some(uri) = line.trim().strip_prefix("keplorayx://") {
                return format!("keplorayx://{uri}");
            }
        }
        panic!("serve-node ended without a pairing URI");
    })
    .await
    .expect("no pairing URI");
    // Keep draining so the node never blocks on a full stdout pipe.
    tokio::spawn(async move { while let Ok(Some(_)) = lines.next_line().await {} });
    (child, uri)
}

async fn wait_for(mut check: impl FnMut() -> bool) {
    for _ in 0..500 {
        if check() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("condition not reached");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn pair_exec_refuse_revoke_over_the_relay() {
    let edge = mock_edge::MockEdge::start().await;
    let url = edge.edge_url();
    let root = tempfile::tempdir().unwrap();
    let (node_dir, laptop_dir, stranger_dir): (PathBuf, PathBuf, PathBuf) =
        (root.path().join("node"), root.path().join("laptop"), root.path().join("stranger"));
    for dir in [&node_dir, &laptop_dir, &stranger_dir] {
        signed_in(dir);
    }

    let (mut node, uri) = serve_node(&url, &node_dir).await;
    let node_id = device_id(&node_dir);
    assert!(uri.contains(&node_id), "{uri}");
    wait_for(|| edge.relay_host_connected(&node_id)).await;

    // Before pairing: the laptop has no trust for this node.
    let out = run(&url, &laptop_dir, &["exec", "--device", &node_id, "--", "true"], b"").await;
    assert_eq!(out.code, 77, "{}", out.stderr);

    // Pair with the URI (what the QR carries).
    let out = run(&url, &laptop_dir, &["pair", "--uri", &uri, "--json"], b"").await;
    assert_eq!(out.code, 0, "{}", out.stderr);
    let reply: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(reply["device"], node_id.as_str());
    let laptop_id = device_id(&laptop_dir);

    // The node lists the laptop; the laptop lists the node as online.
    let out = run(&url, &node_dir, &["paired", "--json"], b"").await;
    let paired: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(paired[0]["id"], laptop_id.as_str());
    assert!(!String::from_utf8_lossy(&out.stdout).contains("proof"), "proof hashes stay private");
    let out = run(&url, &laptop_dir, &["nodes", "--json"], b"").await;
    let nodes: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(nodes[0]["id"], node_id.as_str());
    assert_eq!(nodes[0]["online"], true);

    // stdin in, stdout out, remote exit code back.
    let script = "read l; echo got $l; echo oops >&2; exit 3";
    let out = run(&url, &laptop_dir, &["exec", "--device", &node_id, "--", "sh", "-c", script], b"hello\n").await;
    assert_eq!(out.code, 3, "{}", out.stderr);
    assert_eq!(String::from_utf8_lossy(&out.stdout), "got hello\n");
    assert!(out.stderr.contains("oops"), "{}", out.stderr);

    // Bulk bytes both ways through the credit windows, and EOF arrives.
    let blob: Vec<u8> = (0..3 * 1024 * 1024u32).map(|i| (i * 7 % 251) as u8).collect();
    let out = run(&url, &laptop_dir, &["exec", "--device", &node_id, "--", "cat"], &blob).await;
    assert_eq!(out.code, 0, "{}", out.stderr);
    assert!(out.stdout == blob, "cat round trip differs ({} bytes back)", out.stdout.len());

    // Processes start in the node's home directory... unless the caller names one.
    let out = run(&url, &laptop_dir, &["exec", "--device", &node_id, "--cwd", "/", "--", "pwd"], b"").await;
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "/");

    // The code was single use.
    let out = run(&url, &stranger_dir, &["pair", "--uri", &uri], b"").await;
    assert_eq!(out.code, 77, "{}", out.stderr);

    // A device with a forged trust entry is refused by the node itself.
    let forged = serde_json::json!({ "nodes": [{ "id": node_id, "name": "x", "secret": "00", "pairedAt": 0 }] });
    std::fs::write(stranger_dir.join("nodes.json"), forged.to_string()).unwrap();
    let out =
        run(&url, &stranger_dir, &["exec", "--device", &node_id, "--", "touch", "/tmp/should-not-run"], b"").await;
    assert_eq!(out.code, 77, "{}", out.stderr);
    assert!(out.stderr.contains("not paired"), "{}", out.stderr);

    // Revoke on the node: a running session is cut off and new ones refused.
    let mut long = cmd(&url, &laptop_dir)
        .args(["exec", "--device", &node_id, "--", "sleep", "60"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    tokio::time::sleep(Duration::from_millis(800)).await;
    let out = run(&url, &node_dir, &["revoke", "--device", &laptop_id], b"").await;
    assert_eq!(out.code, 0, "{}", out.stderr);
    let status = tokio::time::timeout(Duration::from_secs(15), long.wait()).await.expect("revoke left session running");
    assert_eq!(status.unwrap().code(), Some(77));
    let out = run(&url, &laptop_dir, &["exec", "--device", &node_id, "--", "true"], b"").await;
    assert_eq!(out.code, 77, "{}", out.stderr);

    // Re-pair with a fresh code, then lose the node mid-run: 255.
    let out = run(&url, &node_dir, &["pair-code", "--json"], b"").await;
    let offer: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let out =
        run(&url, &laptop_dir, &["pair", "--device", &node_id, "--code", offer["code"].as_str().unwrap()], b"").await;
    assert_eq!(out.code, 0, "{}", out.stderr);
    let mut long = cmd(&url, &laptop_dir)
        .args(["exec", "--device", &node_id, "--", "sleep", "60"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    tokio::time::sleep(Duration::from_millis(800)).await;
    node.kill().await.unwrap();
    let status = tokio::time::timeout(Duration::from_secs(15), long.wait()).await.expect("lost node left exec hanging");
    assert_eq!(status.unwrap().code(), Some(255));
    let mut err = String::new();
    long.stderr.take().unwrap().read_to_string(&mut err).await.unwrap();
    assert!(!err.contains("test-token"), "token leaked: {err}");

    // Offline: the laptop still lists it, as offline.
    wait_for(|| !edge.relay_host_connected(&node_id)).await;
    let out = run(&url, &laptop_dir, &["nodes", "--json", "--timeout-ms", "1500"], b"").await;
    let nodes: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(nodes[0]["online"], false);
}
