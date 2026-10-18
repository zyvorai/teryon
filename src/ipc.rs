//! Private local IPC. Unix sockets are preferred; restricted runners can use
//! ephemeral loopback TCP with a random bearer token stored in a 0700 directory.
use crate::{
    protocol::{read_frame, write_frame},
    util,
};
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::{BufReader, Read, Write},
    net::{Shutdown, TcpListener, TcpStream},
    os::unix::{
        fs::PermissionsExt,
        net::{UnixListener, UnixStream},
    },
    time::Duration,
};
#[derive(Serialize, Deserialize)]
pub struct Endpoint {
    pub address: String,
    pub token: String,
}
pub enum Stream {
    Unix(UnixStream),
    Tcp(TcpStream),
}
impl Stream {
    pub fn try_clone(&self) -> std::io::Result<Self> {
        match self {
            Self::Unix(s) => s.try_clone().map(Self::Unix),
            Self::Tcp(s) => s.try_clone().map(Self::Tcp),
        }
    }
    pub fn timeout(&self, d: Duration) -> std::io::Result<()> {
        match self {
            Self::Unix(s) => {
                s.set_read_timeout(Some(d))?;
                s.set_write_timeout(Some(d))
            }
            Self::Tcp(s) => {
                s.set_read_timeout(Some(d))?;
                s.set_write_timeout(Some(d))
            }
        }
    }
    pub fn shutdown_write(&self) {
        let _ = match self {
            Self::Unix(s) => s.shutdown(Shutdown::Write),
            Self::Tcp(s) => s.shutdown(Shutdown::Write),
        };
    }
}
impl Read for Stream {
    fn read(&mut self, b: &mut [u8]) -> std::io::Result<usize> {
        match self {
            Self::Unix(s) => s.read(b),
            Self::Tcp(s) => s.read(b),
        }
    }
}
impl Write for Stream {
    fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
        match self {
            Self::Unix(s) => s.write(b),
            Self::Tcp(s) => s.write(b),
        }
    }
    fn flush(&mut self) -> std::io::Result<()> {
        match self {
            Self::Unix(s) => s.flush(),
            Self::Tcp(s) => s.flush(),
        }
    }
}
pub enum Listener {
    Unix(UnixListener),
    Tcp(TcpListener),
}
impl Listener {
    pub fn bind() -> Result<(Self, String)> {
        let socket = util::socket_path()?;
        if socket.exists() {
            fs::remove_file(&socket)?;
        }
        let force_tcp = std::env::var("TERYON_IPC").is_ok_and(|s| s == "tcp");
        let (listener, address) = if force_tcp {
            let l = TcpListener::bind(("127.0.0.1", 0))?;
            let a = format!("tcp:{}", l.local_addr()?);
            (Self::Tcp(l), a)
        } else {
            match UnixListener::bind(&socket) {
                Ok(l) => {
                    fs::set_permissions(&socket, fs::Permissions::from_mode(0o600))?;
                    (Self::Unix(l), format!("unix:{}", socket.display()))
                }
                Err(e)
                    if [
                        Some(libc::EPERM),
                        Some(libc::EAFNOSUPPORT),
                        Some(libc::ENOTSUP),
                    ]
                    .contains(&e.raw_os_error()) =>
                {
                    let l = TcpListener::bind(("127.0.0.1", 0))?;
                    let a = format!("tcp:{}", l.local_addr()?);
                    (Self::Tcp(l), a)
                }
                Err(e) => return Err(e.into()),
            }
        };
        let token = util::nonce()?;
        let endpoint = Endpoint {
            address,
            token: token.clone(),
        };
        let path = util::runtime_dir()?.join("endpoint.json");
        let temp = path.with_extension(format!("{}.tmp", util::nonce()?));
        util::private_write(&temp, &serde_json::to_vec(&endpoint)?)?;
        fs::rename(temp, path)?;
        Ok((listener, token))
    }
    pub fn accept(&self) -> std::io::Result<Stream> {
        match self {
            Self::Unix(l) => l.accept().map(|(s, _)| Stream::Unix(s)),
            Self::Tcp(l) => l.accept().map(|(s, _)| Stream::Tcp(s)),
        }
    }
}
pub fn connect() -> Result<Stream> {
    let endpoint: Endpoint =
        serde_json::from_slice(&fs::read(util::runtime_dir()?.join("endpoint.json"))?)?;
    let mut stream = if let Some(address) = endpoint.address.strip_prefix("unix:") {
        Stream::Unix(UnixStream::connect(address)?)
    } else if let Some(address) = endpoint.address.strip_prefix("tcp:") {
        let addr: std::net::SocketAddr = address.parse()?;
        if !addr.ip().is_loopback() {
            bail!("IPC endpoint must be loopback");
        }
        Stream::Tcp(TcpStream::connect_timeout(&addr, Duration::from_secs(3))?)
    } else {
        bail!("invalid local IPC endpoint")
    };
    stream.timeout(Duration::from_secs(15))?;
    write_frame(&mut stream, &endpoint.token)?;
    Ok(stream)
}
pub fn authenticate(reader: &mut BufReader<Stream>, expected: &str) -> Result<()> {
    let token: String = read_frame(reader)?.context("IPC authentication missing")?;
    if token.len() != expected.len()
        || token
            .bytes()
            .zip(expected.bytes())
            .fold(0u8, |a, (x, y)| a | (x ^ y))
            != 0
    {
        bail!("IPC authentication failed");
    }
    Ok(())
}
