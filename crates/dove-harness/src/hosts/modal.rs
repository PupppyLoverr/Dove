//! Modal Sandboxes (modal.com/docs/guide/sandbox), driven through the
//! official `modal` Python package (route: exec).
//!
//! Modal's API is gRPC, so Keplorayx does not speak it. The `modal` CLI has
//! no sandbox commands, and `modal container exec` drops stdin without a
//! PTY and always exits 0 (https://modal.com/docs/reference/cli/container),
//! so it can't carry an agent. Instead a short script runs on the Python
//! that `modal` was installed into (read from the `modal` launcher's
//! shebang, else `python3`) and calls the SDK
//! (https://modal.com/docs/reference/modal.Sandbox):
//! `Sandbox.create("sleep", "infinity", app=…)`, `Sandbox.list`,
//! `Sandbox.from_id(…).terminate()`, and for the agent itself
//! `Sandbox.exec(*argv, text=False)` with stdin, stdout and stderr pumped
//! as raw bytes and the exit code passed back. `connect` writes that script
//! into the box's private folder and returns `[python, script, exec, …]`.
//!
//! The key is `MODAL_TOKEN_ID:MODAL_TOKEN_SECRET` (both from
//! https://modal.com/settings/tokens). It reaches the SDK through the
//! environment for list/create/delete; the exec script reads it from
//! `cloud-keys.json` itself, so it is never on a command line.
//!
//! Limits: a sandbox lives at most 24 hours (`timeout=86400`) and is
//! terminated after an hour with no exec running (`idle_timeout`). Modal
//! has no stable pause, so `sleep` refuses: terminating loses the files.
//! Sandboxes live in a Modal app named `keplorayx`.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{Context as _, Result, bail};
use serde_json::Value;

use super::{BoxInfo, BoxSpec, Endpoint, Keys, Probe, Provider};

pub struct Modal;
pub static PROVIDER: Modal = Modal;

/// Run as `python -c SCRIPT <op> <keys-file|""> <args…>` or from a file.
const SCRIPT: &str = r#"
import json, os, sys, threading
op, keys_path = sys.argv[1], sys.argv[2]
if keys_path and not os.environ.get("MODAL_TOKEN_ID"):
    try:
        raw = json.load(open(keys_path)).get("keys", {}).get("modal", "")
        tid, sec = raw.strip().split(":", 1)
        os.environ["MODAL_TOKEN_ID"], os.environ["MODAL_TOKEN_SECRET"] = tid.strip(), sec.strip()
    except Exception:
        pass
import modal
APP = "keplorayx"

def info(sb):
    code = sb.poll()
    name = sb.object_id
    try:
        name = sb.get_tags().get("name", name)
    except Exception:
        pass
    return {"id": sb.object_id, "name": name, "running": code is None,
            "state": "running" if code is None else "exited %s" % code}

if op == "list":
    try:
        app = modal.App.lookup(APP)
    except modal.exception.NotFoundError:
        print("[]")
        sys.exit(0)
    print(json.dumps([info(sb) for sb in modal.Sandbox.list(app_id=app.app_id)]))
elif op == "create":
    name, image = sys.argv[3], sys.argv[4]
    app = modal.App.lookup(APP, create_if_missing=True)
    img = modal.Image.from_registry(image) if image else modal.Image.debian_slim().apt_install("git", "curl")
    sb = modal.Sandbox.create("sleep", "infinity", app=app, image=img, timeout=86400, idle_timeout=3600)
    if name:
        try:
            sb.set_tags({"name": name})
        except Exception:
            pass
    print(json.dumps(info(sb)))
elif op == "status":
    print(json.dumps(info(modal.Sandbox.from_id(sys.argv[3]))))
elif op == "terminate":
    modal.Sandbox.from_id(sys.argv[3]).terminate()
    print("{}")
elif op == "exec":
    sb = modal.Sandbox.from_id(sys.argv[3])
    p = sb.exec(*sys.argv[4:], text=False)
    def pump_out(src, dst):
        try:
            for chunk in src:
                dst.write(chunk if isinstance(chunk, bytes) else chunk.encode())
                dst.flush()
        except Exception:
            pass
    def pump_in():
        try:
            while True:
                b = sys.stdin.buffer.read1(65536)
                if not b:
                    break
                p.stdin.write(b)
                p.stdin.drain()
            p.stdin.write_eof()
            p.stdin.drain()
        except Exception:
            pass
    threading.Thread(target=pump_in, daemon=True).start()
    outs = [threading.Thread(target=pump_out, args=(p.stdout, sys.stdout.buffer)),
            threading.Thread(target=pump_out, args=(p.stderr, sys.stderr.buffer))]
    for t in outs:
        t.start()
    code = p.wait()
    for t in outs:
        t.join()
    sys.stdout.flush()
    sys.stderr.flush()
    os._exit(code if isinstance(code, int) and 0 <= code < 256 else 1)
else:
    sys.exit("unknown operation " + op)
"#;

/// `MODAL_TOKEN_ID:MODAL_TOKEN_SECRET`.
fn parse_key(raw: &str) -> Option<(&str, &str)> {
    let (id, secret) = raw.trim().split_once(':')?;
    let (id, secret) = (id.trim(), secret.trim());
    (!id.is_empty() && !secret.is_empty()).then_some((id, secret))
}

/// The interpreter `modal` runs on: the launcher script's shebang when it
/// names an existing file (pipx, venvs), else `python3`/`python`.
fn python() -> Result<PathBuf> {
    let launcher = which::which("modal").context("the Modal CLI is not installed: run `pip install modal`")?;
    if let Some(p) = std::fs::read(&launcher).ok().and_then(|b| shebang_python(&b)) {
        return Ok(p);
    }
    which::which("python3").or_else(|_| which::which("python")).context("no python3 on PATH for the Modal SDK")
}

fn shebang_python(script: &[u8]) -> Option<PathBuf> {
    let first = script.split(|&b| b == b'\n').next()?;
    let line = std::str::from_utf8(first).ok()?.strip_prefix("#!")?.trim();
    // `#!/usr/bin/env python3` names no file; leave it to the PATH lookup.
    let path = PathBuf::from(line.split_whitespace().next()?);
    (path.is_file() && path.file_name()?.to_string_lossy().starts_with("python")).then_some(path)
}

impl Modal {
    /// Run one SDK operation and parse its JSON output.
    fn run(&self, keys: &Keys, args: &[&str]) -> Result<Value> {
        let mut cmd = Command::new(python()?);
        cmd.arg("-c").arg(SCRIPT).args(args).stdin(Stdio::null());
        if let Some((id, secret)) = keys.get(self.id()).and_then(parse_key) {
            cmd.env("MODAL_TOKEN_ID", id).env("MODAL_TOKEN_SECRET", secret);
        }
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(0x0800_0000);
        }
        let out = cmd.output().context("running the Modal SDK")?;
        if !out.status.success() {
            bail!("Modal: {}", last_line(&String::from_utf8_lossy(&out.stderr)));
        }
        let text = String::from_utf8_lossy(&out.stdout);
        let json = text.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("null");
        serde_json::from_str(json).context("reading the Modal SDK's output")
    }
}

impl Provider for Modal {
    fn id(&self) -> &'static str {
        "modal"
    }

    fn label(&self) -> &'static str {
        "Modal"
    }

    fn key_url(&self) -> &'static str {
        "https://modal.com/settings/tokens"
    }

    fn verified(&self) -> bool {
        false
    }

    fn needs_key(&self) -> bool {
        true
    }

    fn probe(&self, keys: &Keys) -> Probe {
        if which::which("modal").is_err() {
            return Probe::NeedsSetup("Install the Modal CLI and SDK: `pip install modal`".into());
        }
        match keys.get(self.id()) {
            None => Probe::NeedsSetup("Add your Modal token as MODAL_TOKEN_ID:MODAL_TOKEN_SECRET".into()),
            Some(raw) if parse_key(raw).is_none() => {
                Probe::NeedsSetup("The Modal key must look like ak-…:as-… (token id, colon, secret)".into())
            }
            Some(_) => Probe::Ready,
        }
    }

    fn list(&self, keys: &Keys) -> Result<Vec<BoxInfo>> {
        Ok(parse_boxes(&self.run(keys, &["list", ""])?))
    }

    fn create(&self, keys: &Keys, spec: &BoxSpec) -> Result<BoxInfo> {
        let v = self
            .run(keys, &["create", "", spec.name.as_deref().unwrap_or(""), spec.template.as_deref().unwrap_or("")])?;
        box_info(&v).context("Modal did not say which sandbox it created")
    }

    fn wake(&self, keys: &Keys, id: &str) -> Result<()> {
        let b = box_info(&self.run(keys, &["status", "", id])?).context("unreadable Modal sandbox status")?;
        if !b.running {
            bail!("Modal sandbox {id} has {}; Modal can't restart it, so create a new one", b.state);
        }
        Ok(())
    }

    fn sleep(&self, _keys: &Keys, _id: &str) -> Result<()> {
        bail!("Modal sandboxes can't be paused; delete it to stop paying (its files are lost)")
    }

    fn delete(&self, keys: &Keys, id: &str) -> Result<()> {
        self.run(keys, &["terminate", "", id]).map(|_| ())
    }

    fn connect(&self, _keys: &Keys, id: &str, dir: &Path) -> Result<Endpoint> {
        let python = python()?;
        let script = dir.join("modal_exec.py");
        super::write_private(&script, SCRIPT.as_bytes())?;
        Ok(Endpoint::Exec { argv: exec_argv(&python, &script, &Keys::path(), id) })
    }
}

fn exec_argv(python: &Path, script: &Path, keys_file: &Path, id: &str) -> Vec<String> {
    vec![
        python.display().to_string(),
        script.display().to_string(),
        "exec".into(),
        keys_file.display().to_string(),
        id.to_string(),
    ]
}

fn box_info(v: &Value) -> Option<BoxInfo> {
    let id = v["id"].as_str()?.to_string();
    Some(BoxInfo {
        name: v["name"].as_str().unwrap_or(&id).to_string(),
        state: v["state"].as_str().unwrap_or("unknown").to_string(),
        running: v["running"].as_bool().unwrap_or(false),
        id,
    })
}

fn parse_boxes(v: &Value) -> Vec<BoxInfo> {
    v.as_array().into_iter().flatten().filter_map(box_info).collect()
}

/// The last non-empty line of a Python traceback (the exception itself).
fn last_line(stderr: &str) -> &str {
    stderr.lines().rev().map(str::trim).find(|l| !l.is_empty()).unwrap_or("the SDK failed")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn key_is_id_colon_secret() {
        assert_eq!(parse_key(" ak-1 : as-2 "), Some(("ak-1", "as-2")));
        assert_eq!(parse_key("ak-1"), None);
        assert_eq!(parse_key("ak-1:"), None);
    }

    #[test]
    fn reads_sdk_output() {
        let v = json!([
            {"id": "sb-1", "name": "work", "running": true, "state": "running"},
            {"id": "sb-2", "running": false, "state": "exited 0"},
            {"name": "bad"}
        ]);
        let b = parse_boxes(&v);
        assert_eq!(b.len(), 2);
        assert_eq!(b[0], BoxInfo { id: "sb-1".into(), name: "work".into(), state: "running".into(), running: true });
        assert_eq!((b[1].name.as_str(), b[1].running), ("sb-2", false));
        assert_eq!(last_line("Traceback…\n  File x\nAuthError: bad token\n\n"), "AuthError: bad token");
    }

    #[test]
    fn exec_argv_keeps_secrets_off_the_command_line() {
        let a = exec_argv(Path::new("/py"), Path::new("/d/modal_exec.py"), Path::new("/h/cloud-keys.json"), "sb-1");
        assert_eq!(a, ["/py", "/d/modal_exec.py", "exec", "/h/cloud-keys.json", "sb-1"].map(String::from).to_vec());
        assert!(SCRIPT.contains("text=False") && SCRIPT.contains("write_eof"));
    }

    #[test]
    fn shebangs_name_an_interpreter_only_when_it_exists() {
        assert_eq!(shebang_python(b"#!/usr/bin/env python3\nimport x"), None);
        assert_eq!(shebang_python(b"#!/definitely/missing/python3\n"), None);
        assert_eq!(shebang_python(b"MZ\x90\x00"), None);
    }
}
