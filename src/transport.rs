use crate::{daemon, ipc, protocol::*, util};
use anyhow::{bail, Context, Result};
use std::io::{BufReader, Read, Write};
use std::process::{Child, Command, Stdio};

#[derive(Clone, Debug)]
pub struct Target {
    pub host: String,
    pub remote_bin: Option<String>,
}
impl Target {
    pub fn new(host: String, remote_bin: Option<String>) -> Result<Self> {
        if host.is_empty()
            || host.len() > 253
            || host.starts_with('-')
            || !host
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"_.@-:[]".contains(&b))
        {
            bail!("invalid SSH destination; use a host alias or user@host");
        }
        if remote_bin
            .as_ref()
            .is_some_and(|s| !s.starts_with('/') || s.contains('\0') || s.contains('\n'))
        {
            bail!("--remote-bin must be an absolute remote path");
        }
        Ok(Self { host, remote_bin })
    }
    pub fn local(&self) -> bool {
        self.host == "local"
    }
    fn ssh_base(&self) -> Command {
        let mut cmd = Command::new("ssh");
        if let Some(path) = std::env::var_os("TERYON_SSH_CONFIG") {
            cmd.arg("-F").arg(path);
        }
        cmd.args([
            "-T",
            "-o",
            "ConnectTimeout=8",
            "-o",
            "ServerAliveInterval=5",
            "-o",
            "ServerAliveCountMax=3",
            "-o",
            "BatchMode=yes",
            "-o",
            "StrictHostKeyChecking=yes",
        ]);
        cmd
    }
    pub fn ssh(&self) -> Command {
        let mut cmd = self.ssh_base();
        cmd.arg(&self.host);
        cmd
    }
    fn remote_executable(&self) -> String {
        self.remote_bin
            .as_ref()
            .map(|s| util::shell_quote(s))
            .unwrap_or_else(|| "\"$HOME/.local/bin/teryon\"".into())
    }
    pub fn connect(&self) -> Result<Rpc> {
        let mut rpc = if self.local() {
            daemon::ensure()?;
            let stream = ipc::connect()?;
            Rpc {
                reader: BufReader::new(Box::new(stream.try_clone()?)),
                writer: Box::new(stream),
                child: None,
                instance: String::new(),
            }
        } else {
            let mut child = self
                .ssh()
                .arg(format!("exec {} relay", self.remote_executable()))
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::inherit())
                .spawn()
                .context("start OpenSSH")?;
            let reader = child.stdout.take().context("SSH stdout missing")?;
            let writer = child.stdin.take().context("SSH stdin missing")?;
            Rpc {
                reader: BufReader::new(Box::new(reader)),
                writer: Box::new(writer),
                child: Some(child),
                instance: String::new(),
            }
        };
        match rpc.call(Request::Hello { version: VERSION })? {
            Response::Hello {
                version: VERSION,
                instance,
                ..
            } => {
                rpc.instance = instance;
                Ok(rpc)
            }
            _ => bail!("unexpected handshake response"),
        }
    }
    /// Deploy only an explicitly supplied binary, after checking target architecture.
    pub fn deploy(&self, binary: &std::path::Path) -> Result<()> {
        if self.local() {
            bail!("deploy expects an SSH host, not 'local'");
        }
        let binary = binary
            .canonicalize()
            .context("deployment binary does not exist")?;
        let mut probe = self.ssh();
        let result = probe.arg("uname -s; uname -m").output()?;
        if !result.status.success() {
            bail!("could not identify the SSH target");
        }
        eprintln!(
            "Target platform: {}",
            String::from_utf8_lossy(&result.stdout)
                .trim()
                .replace('\n', " / ")
        );
        let token = util::nonce()?;
        let temp = format!("teryon-upload-{token}");
        let mut scp = Command::new("scp");
        if let Some(path) = std::env::var_os("TERYON_SSH_CONFIG") {
            scp.arg("-F").arg(path);
        }
        let status = scp
            .args([
                "-q",
                "-o",
                "BatchMode=yes",
                "-o",
                "StrictHostKeyChecking=yes",
                "--",
            ])
            .arg(&binary)
            .arg(format!("{}:{temp}", self.host))
            .status()?;
        if !status.success() {
            bail!("SCP upload failed");
        }
        let destination = self
            .remote_bin
            .as_ref()
            .map(|p| util::shell_quote(p))
            .unwrap_or_else(|| "\"$HOME/.local/bin/teryon\"".into());
        // The uploaded binary is validated before replacing an installation.
        let script = format!("set -eu\ntrap 'rm -f {temp}' EXIT\nchmod 700 {temp}\n./{temp} --version\nmkdir -p \"$HOME/.local/bin\"\nmv -f {temp} {destination}\n{destination} doctor local");
        let status = self.ssh().arg(script).status()?;
        if !status.success() {
            bail!("remote deployment verification failed (supply a binary for the target OS and architecture)");
        }
        Ok(())
    }
    pub fn forward(&self, local: u16, remote: u16) -> Result<()> {
        if self.local() {
            bail!("port forwarding expects an SSH host");
        }
        if local == 0 || remote == 0 {
            bail!("ports must be between 1 and 65535");
        }
        let status = self
            .ssh_base()
            .args(["-N", "-o", "ExitOnForwardFailure=yes", "-L"])
            .arg(format!("127.0.0.1:{local}:127.0.0.1:{remote}"))
            .arg(&self.host)
            .status()?;
        if !status.success() {
            bail!("SSH forwarding ended with {status}");
        }
        Ok(())
    }
    pub fn transfer(
        &self,
        upload: bool,
        source: &str,
        destination: &str,
        limit_kbps: Option<u32>,
    ) -> Result<()> {
        if self.local() {
            bail!("file transfer expects an SSH host");
        }
        let remote = if upload { destination } else { source };
        if remote.starts_with('-') || remote.contains(['\n', '\0']) {
            bail!("invalid remote path");
        }
        // --protect-args prevents the remote shell from interpreting filenames.
        // --partial-dir preserves interrupted transfers without in-place corruption.
        let mut cmd = Command::new("rsync");
        let mut ssh="ssh -o BatchMode=yes -o StrictHostKeyChecking=yes -o ConnectTimeout=8 -o ServerAliveInterval=5 -o ServerAliveCountMax=3".to_string();
        if let Some(path) = std::env::var_os("TERYON_SSH_CONFIG") {
            ssh.push_str(" -F ");
            ssh.push_str(&util::shell_quote(&path.to_string_lossy()));
        }
        cmd.args([
            "-a",
            "--protect-args",
            "--partial-dir=.teryon-partial",
            "--progress",
            "-e",
            &ssh,
        ]);
        if let Some(limit) = limit_kbps {
            if limit == 0 {
                bail!("bandwidth limit must be positive");
            }
            cmd.arg(format!("--bwlimit={limit}"));
        }
        cmd.arg("--");
        if upload {
            cmd.arg(source).arg(format!("{}:{destination}", self.host));
        } else {
            cmd.arg(format!("{}:{source}", self.host)).arg(destination);
        }
        if !cmd
            .status()
            .context("run rsync (install rsync 3+ on both machines)")?
            .success()
        {
            bail!("transfer interrupted or failed; repeat the same command to resume");
        }
        Ok(())
    }
}
pub struct Rpc {
    pub instance: String,
    reader: BufReader<Box<dyn Read + Send>>,
    writer: Box<dyn Write + Send>,
    child: Option<Child>,
}
impl Rpc {
    pub fn call(&mut self, request: Request) -> Result<Response> {
        write_frame(&mut self.writer, &request)?;
        let response = read_frame(&mut self.reader)?.context("connection closed")?;
        if let Response::Error { message } = response {
            bail!("server: {message}");
        }
        Ok(response)
    }
}
impl Drop for Rpc {
    fn drop(&mut self) {
        if let Some(child) = self.child.as_mut() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn destinations_are_not_options_or_commands() {
        for bad in [
            "-oProxyCommand=evil",
            "mini;whoami",
            "mini\nfoo",
            "$(id)",
            "",
        ] {
            assert!(Target::new(bad.into(), None).is_err());
        }
        for good in ["local", "mini", "susant@mini.local", "[::1]"] {
            assert!(Target::new(good.into(), None).is_ok());
        }
    }
}
