use crate::{
    protocol::{Command, Request, Snapshot},
    runtime::{self, Endpoint},
    PROTOCOL_VERSION,
};
use anyhow::{bail, Context, Result};
#[cfg(not(windows))]
use std::process::{Command as Process, Stdio};
use std::{path::Path, time::Duration};

#[derive(Clone)]
pub struct Client {
    http: reqwest::Client,
    pub endpoint: Endpoint,
}
impl Client {
    pub fn new(endpoint: Endpoint) -> Result<Self> {
        Ok(Self {
            http: reqwest::Client::builder()
                .no_proxy()
                .timeout(Duration::from_secs(5))
                .build()?,
            endpoint,
        })
    }
    pub async fn discover() -> Result<Self> {
        let client = Self::new(Endpoint::read(&runtime::runtime_dir()?)?)?;
        client.snapshot().await?;
        Ok(client)
    }
    async fn decode(response: reqwest::Response) -> Result<Snapshot> {
        if !response.status().is_success() {
            let status = response.status();
            let body: crate::protocol::ApiError =
                response.json().await.context("invalid THMR response")?;
            bail!("{} ({status})", body.error);
        }
        let snapshot: Snapshot = response.json().await?;
        if snapshot.protocol_version != PROTOCOL_VERSION {
            bail!("incompatible THMR protocol");
        }
        Ok(snapshot)
    }
    pub async fn snapshot(&self) -> Result<Snapshot> {
        let state = Self::decode(
            self.http
                .get(self.endpoint.url("/v1/state"))
                .header("X-THMR-Protocol", PROTOCOL_VERSION)
                .bearer_auth(&self.endpoint.token)
                .send()
                .await?,
        )
        .await?;
        if state.instance_id != self.endpoint.instance_id {
            bail!("THMR instance changed; reconnect required");
        }
        Ok(state)
    }
    pub async fn execute(&self, command: Command) -> Result<Snapshot> {
        Self::decode(
            self.http
                .post(self.endpoint.url("/v1/command"))
                .bearer_auth(&self.endpoint.token)
                .json(&Request {
                    protocol_version: PROTOCOL_VERSION,
                    command,
                })
                .send()
                .await?,
        )
        .await
    }
    pub async fn ensure() -> Result<Self> {
        if let Ok(endpoint) = Endpoint::read(&runtime::runtime_dir()?) {
            let client = Self::new(endpoint)?;
            if client.snapshot().await.is_ok() {
                return Ok(client);
            }
        } else if runtime::runtime_dir()?.join("instance.json").exists() {
            // A version mismatch is actionable: never silently start a second player.
            Endpoint::read(&runtime::runtime_dir()?)?;
        }
        let daemon = daemon_path()?;
        spawn_detached(&daemon, &[])?;
        for _ in 0..100 {
            tokio::time::sleep(Duration::from_millis(100)).await;
            if let Ok(client) = Self::discover().await {
                return Ok(client);
            }
        }
        bail!("THMR host did not become ready; run thmrd in a terminal or thmr doctor for diagnostics")
    }
}

pub fn daemon_path() -> Result<std::path::PathBuf> {
    let exe = std::env::current_exe()?;
    let dir = exe.parent().context("cannot locate THMR installation")?;
    let mut candidates = vec![dir.join(if cfg!(windows) { "thmrd.exe" } else { "thmrd" })];
    if cfg!(target_os = "macos") {
        candidates.push(dir.join("../THMRHost.app/Contents/MacOS/thmrd"));
    }
    candidates
        .into_iter()
        .find(|path| path.is_file())
        .context("thmrd not found; install the complete terminal package")
}

pub fn spawn_detached(path: &Path, args: &[&str]) -> Result<()> {
    #[cfg(windows)]
    {
        spawn_windows(path, args)
    }
    #[cfg(not(windows))]
    {
        let mut process = Process::new(path);
        process
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            // Start a new login-user process session, independent of the invoking terminal.
            unsafe {
                process.pre_exec(|| {
                    unsafe extern "C" {
                        fn setsid() -> i32;
                    }
                    if setsid() < 0 {
                        return Err(std::io::Error::last_os_error());
                    }
                    Ok(())
                });
            }
        }
        let child = process
            .spawn()
            .with_context(|| format!("could not start {}", path.display()))?;
        // Reap if the launching CLI stays alive (e.g. TUI); detached hosts survive its exit.
        std::thread::spawn(move || {
            let mut child = child;
            let _ = child.wait();
        });
        Ok(())
    }
}

#[cfg(windows)]
fn spawn_windows(path: &Path, args: &[&str]) -> Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::{
        Foundation::CloseHandle,
        System::Threading::{CreateProcessW, CREATE_NO_WINDOW, PROCESS_INFORMATION, STARTUPINFOW},
    };
    fn quote(value: &str) -> String {
        let mut result = String::from("\"");
        let mut slashes = 0;
        for c in value.chars() {
            if c == '\\' {
                slashes += 1;
                continue;
            }
            result.push_str(&"\\".repeat(if c == '"' { slashes * 2 + 1 } else { slashes }));
            result.push(c);
            slashes = 0;
        }
        result.push_str(&"\\".repeat(slashes * 2));
        result.push('"');
        result
    }
    let application: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    let mut command = quote(&path.to_string_lossy());
    for arg in args {
        command.push(' ');
        command.push_str(&quote(arg));
    }
    let mut command: Vec<u16> = command.encode_utf16().chain(Some(0)).collect();
    let startup = STARTUPINFOW {
        cb: std::mem::size_of::<STARTUPINFOW>() as u32,
        ..unsafe { std::mem::zeroed() }
    };
    let mut info: PROCESS_INFORMATION = unsafe { std::mem::zeroed() };
    // Do not inherit any caller pipe/file handles. The host is independent of CLI capture.
    let success = unsafe {
        CreateProcessW(
            application.as_ptr(),
            command.as_mut_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            0,
            CREATE_NO_WINDOW,
            std::ptr::null(),
            std::ptr::null(),
            &startup,
            &mut info,
        )
    };
    if success == 0 {
        return Err(std::io::Error::last_os_error())
            .with_context(|| format!("could not start {}", path.display()));
    }
    unsafe {
        CloseHandle(info.hThread);
        CloseHandle(info.hProcess);
    }
    Ok(())
}
