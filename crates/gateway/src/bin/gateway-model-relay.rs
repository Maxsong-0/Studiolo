//! Linux-only dedicated-credential key-holder; no Vault or control database is opened.
#[cfg(target_os = "linux")]
mod linux {
    use serde::Deserialize;
    use std::{
        fs,
        os::unix::{
            fs::{MetadataExt, PermissionsExt},
            net::UnixListener,
        },
        path::{Path, PathBuf},
    };
    use studiolo_gateway::{HttpsRelayTransport, ProviderCredential, serve_key_holder_connection};
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Config {
        schema: u32,
        socket: PathBuf,
        client_uid: u32,
        anthropic_key_file: PathBuf,
        openai_key_file: PathBuf,
    }
    fn private(path: &Path) -> Result<Vec<u8>, ()> {
        if !path.is_absolute() || fs::canonicalize(path).map_err(|_| ())? != path {
            return Err(());
        }
        let m = fs::symlink_metadata(path).map_err(|_| ())?;
        if !m.is_file()
            || m.nlink() != 1
            || m.uid() != rustix::process::geteuid().as_raw()
            || m.mode() & 0o777 != 0o600
            || m.len() > 16384
        {
            return Err(());
        }
        fs::read(path).map_err(|_| ())
    }
    pub fn run() -> Result<(), ()> {
        let residency = studiolo_process_safety::harden().map_err(|_| ())?;
        use std::io::Write;
        writeln!(std::io::stderr(), "秘密驻留面：{residency}").map_err(|_| ())?;
        let path = std::env::args_os().nth(1).ok_or(())?;
        let config: Config = serde_json::from_slice(&private(Path::new(&path))?).map_err(|_| ())?;
        if config.schema != 1
            || config.client_uid == 0
            || rustix::process::geteuid().as_raw() == 0
            || config.client_uid == rustix::process::geteuid().as_raw()
        {
            return Err(());
        }
        let parent = config.socket.parent().ok_or(())?;
        let m = fs::symlink_metadata(parent).map_err(|_| ())?;
        if !m.is_dir() || m.uid() != rustix::process::geteuid().as_raw() || m.mode() & 0o027 != 0 {
            return Err(());
        }
        // A stale socket is an operator recovery action, not an automatic unlink.
        let listener = UnixListener::bind(&config.socket).map_err(|_| ())?;
        fs::set_permissions(&config.socket, fs::Permissions::from_mode(0o660)).map_err(|_| ())?;
        let load = |path: &Path| -> Result<ProviderCredential, ()> {
            let bytes = zeroize::Zeroizing::new(private(path)?);
            let text = std::str::from_utf8(&bytes).map_err(|_| ())?;
            ProviderCredential::new(text.trim().to_owned()).map_err(|_| ())
        };
        let transport = HttpsRelayTransport::new(
            load(&config.anthropic_key_file)?,
            load(&config.openai_key_file)?,
        );
        for stream in listener.incoming() {
            let Ok(stream) = stream else {
                return Err(());
            };
            // Single upstream operation at a time; no unbounded worker/thread creation.
            let _ = serve_key_holder_connection(&transport, stream, config.client_uid);
        }
        Ok(())
    }
}
fn main() {
    #[cfg(target_os = "linux")]
    if linux::run().is_ok() {
        return;
    }
    eprintln!("dedicated Gateway key-holder unavailable");
    std::process::exit(1);
}
