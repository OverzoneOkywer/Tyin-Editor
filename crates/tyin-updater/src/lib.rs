use anyhow::{bail, Context, Result};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const RELEASE_API: &str = "https://api.github.com/repos/OverzoneOkywer/Tyin-Editor/releases/latest";
const RELEASE_ASSET_PREFIX: &str =
    "https://github.com/OverzoneOkywer/Tyin-Editor/releases/download/";
const MAX_PACKAGE_BYTES: u64 = 256 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Release {
    pub version: String,
    pub notes: String,
    asset_name: String,
    download_url: String,
    sha256: [u8; 32],
    size: u64,
}

#[derive(Deserialize)]
struct ApiRelease {
    tag_name: String,
    body: Option<String>,
    assets: Vec<ApiAsset>,
}

#[derive(Deserialize)]
struct ApiAsset {
    name: String,
    browser_download_url: String,
    digest: Option<String>,
    size: u64,
}

pub fn check_for_update() -> Result<Option<Release>> {
    let response = agent()
        .get(RELEASE_API)
        .set("Accept", "application/vnd.github+json")
        .set("X-GitHub-Api-Version", "2022-11-28")
        .set("User-Agent", "Tyin-Editor-Updater")
        .call()
        .context("failed to query the latest Tyin-Editor release")?;
    let release: ApiRelease = serde_json::from_reader(response.into_reader())
        .context("GitHub returned invalid release metadata")?;
    let version = release
        .tag_name
        .strip_prefix('v')
        .unwrap_or(&release.tag_name)
        .to_owned();
    let latest_version = parse_version(&version)
        .with_context(|| format!("unsupported release version: {}", release.tag_name))?;
    let current_version =
        parse_version(env!("CARGO_PKG_VERSION")).context("application version is invalid")?;
    if latest_version <= current_version {
        return Ok(None);
    }

    let asset_name = platform_asset_name(&version)?;
    let asset = release
        .assets
        .into_iter()
        .find(|asset| asset.name == asset_name)
        .with_context(|| format!("release {} has no {asset_name} installer", release.tag_name))?;
    if asset.size == 0 || asset.size > MAX_PACKAGE_BYTES {
        bail!("release installer has an invalid or unsupported file size");
    }
    if !asset.browser_download_url.starts_with(RELEASE_ASSET_PREFIX) {
        bail!("release installer URL is outside the official GitHub repository");
    }
    let sha256 = parse_digest(asset.digest.as_deref())?;

    Ok(Some(Release {
        version,
        notes: release.body.unwrap_or_default(),
        asset_name,
        download_url: asset.browser_download_url,
        sha256,
        size: asset.size,
    }))
}

pub fn download_release(release: &Release) -> Result<PathBuf> {
    let response = agent()
        .get(&release.download_url)
        .set("User-Agent", "Tyin-Editor-Updater")
        .call()
        .context("failed to download the update installer")?;
    let (path, mut file) = create_update_file(&release.asset_name)?;
    let result = (|| {
        let mut reader = response.into_reader().take(MAX_PACKAGE_BYTES + 1);
        let mut hasher = Sha256::new();
        let mut total = 0_u64;
        let mut buffer = [0; 32 * 1024];
        loop {
            let count = reader
                .read(&mut buffer)
                .context("failed to read update data")?;
            if count == 0 {
                break;
            }
            total += count as u64;
            if total > MAX_PACKAGE_BYTES || total > release.size {
                bail!("downloaded installer exceeds its published size");
            }
            hasher.update(&buffer[..count]);
            file.write_all(&buffer[..count])
                .context("failed to save update installer")?;
        }
        if total != release.size {
            bail!("downloaded installer size does not match GitHub release metadata");
        }
        let digest: [u8; 32] = hasher.finalize().into();
        if digest != release.sha256 {
            bail!("downloaded installer failed SHA-256 verification");
        }
        file.flush().context("failed to flush update installer")?;
        Ok(())
    })();

    if let Err(error) = result {
        drop(file);
        let _ = fs::remove_file(&path);
        return Err(error);
    }
    drop(file);
    Ok(path)
}

pub fn launch_installer(installer: &Path, app: &Path) -> Result<()> {
    if !installer.is_file() || !app.is_file() {
        bail!("update installer or application executable is missing");
    }
    let parent_pid = std::process::id();

    #[cfg(target_os = "windows")]
    {
        launch_windows_installer(installer, app, parent_pid)
    }
    #[cfg(target_os = "linux")]
    {
        launch_linux_installer(installer, app, parent_pid)
    }
    #[cfg(not(any(target_os = "windows", target_os = "linux")))]
    {
        let _ = (installer, app, parent_pid);
        bail!("in-app updates are supported on Windows and Linux only")
    }
}

pub fn installed_app_path() -> Result<PathBuf> {
    let executable = std::env::current_exe().context("failed to locate Tyin-Editor executable")?;
    #[cfg(target_os = "linux")]
    {
        let expected = Path::new("/usr/bin/tyin-editor");
        if executable == expected && executable.is_file() {
            return Ok(executable);
        }
        bail!("install Tyin-Editor from the official Debian package to enable updates")
    }
    #[cfg(target_os = "windows")]
    {
        let local_app_data = std::env::var_os("LOCALAPPDATA")
            .context("Windows user application directory is unavailable")?;
        let expected = PathBuf::from(local_app_data)
            .join("Programs")
            .join("Tyin-Editor")
            .join("tyin-app.exe");
        let executable = executable
            .canonicalize()
            .context("failed to resolve Tyin-Editor executable path")?;
        let expected = expected
            .canonicalize()
            .context("install Tyin-Editor from the official Windows installer to enable updates")?;
        if executable
            .to_string_lossy()
            .eq_ignore_ascii_case(&expected.to_string_lossy())
        {
            return Ok(executable);
        }
        bail!("install Tyin-Editor from the official Windows installer to enable updates")
    }
    #[cfg(not(any(target_os = "windows", target_os = "linux")))]
    {
        let _ = executable;
        bail!("in-app updates are supported on Windows and Linux only")
    }
}

fn agent() -> ureq::Agent {
    ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(10))
        .timeout_read(Duration::from_secs(60))
        .timeout_write(Duration::from_secs(60))
        .build()
}

fn parse_version(version: &str) -> Result<[u64; 3]> {
    let mut parts = version.split('.');
    let parsed = [
        parts.next().context("version is missing major component")?,
        parts.next().context("version is missing minor component")?,
        parts.next().context("version is missing patch component")?,
    ];
    if parts.next().is_some() {
        bail!("version must have exactly three numeric components");
    }
    let mut result = [0; 3];
    for (index, component) in parsed.into_iter().enumerate() {
        if component.is_empty() || !component.bytes().all(|byte| byte.is_ascii_digit()) {
            bail!("version components must be numeric");
        }
        result[index] = component
            .parse()
            .context("version component is too large")?;
    }
    Ok(result)
}

fn platform_asset_name(version: &str) -> Result<String> {
    #[cfg(target_os = "windows")]
    {
        Ok(format!("Tyin-Editor-Setup-{version}-x64.exe"))
    }
    #[cfg(target_os = "linux")]
    {
        Ok(format!("tyin-editor_{version}_amd64.deb"))
    }
    #[cfg(not(any(target_os = "windows", target_os = "linux")))]
    {
        let _ = version;
        bail!("in-app updates are supported on Windows and Linux only")
    }
}

fn parse_digest(digest: Option<&str>) -> Result<[u8; 32]> {
    let value = digest
        .and_then(|digest| digest.strip_prefix("sha256:"))
        .context("release installer has no GitHub SHA-256 digest")?;
    if value.len() != 64 {
        bail!("release installer SHA-256 digest has an invalid length");
    }
    let mut bytes = [0; 32];
    for (index, byte) in bytes.iter_mut().enumerate() {
        let offset = index * 2;
        *byte = u8::from_str_radix(&value[offset..offset + 2], 16)
            .context("release installer SHA-256 digest is malformed")?;
    }
    Ok(bytes)
}

fn create_update_file(asset_name: &str) -> Result<(PathBuf, File)> {
    let suffix = Path::new(asset_name)
        .extension()
        .and_then(|extension| extension.to_str())
        .context("release installer has no file extension")?;
    let directory = std::env::temp_dir();
    for attempt in 0..10 {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let path = directory.join(format!(
            "tyin-editor-update-{}-{nonce}-{attempt}.{suffix}",
            std::process::id()
        ));
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(file) => return Ok((path, file)),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error).context("failed to create update file"),
        }
    }
    bail!("could not allocate a unique update installer file")
}

#[cfg(target_os = "linux")]
fn launch_linux_installer(installer: &Path, app: &Path, parent_pid: u32) -> Result<()> {
    const SCRIPT: &str = r#"
parent_pid=$1
installer=$2
application=$3
while kill -0 "$parent_pid" 2>/dev/null; do sleep 0.25; done
pkexec /usr/bin/apt install -y "$installer"
status=$?
rm -f -- "$installer"
"$application"
exit "$status"
"#;
    Command::new("sh")
        .arg("-c")
        .arg(SCRIPT)
        .arg("tyin-editor-updater")
        .arg(parent_pid.to_string())
        .arg(installer)
        .arg(app)
        .spawn()
        .context("failed to start the update installer")
        .map(|_| ())
}

#[cfg(target_os = "windows")]
fn launch_windows_installer(installer: &Path, app: &Path, parent_pid: u32) -> Result<()> {
    use std::os::windows::process::CommandExt;

    let (script_path, mut script_file) = create_update_file("updater.ps1")?;
    let installer = encode_path(installer)?;
    let app = encode_path(app)?;
    let script = format!(
        r#"$ErrorActionPreference = 'Stop'
$installer = [Text.Encoding]::UTF8.GetString([Convert]::FromBase64String('{installer}'))
$application = [Text.Encoding]::UTF8.GetString([Convert]::FromBase64String('{app}'))
try {{
  Wait-Process -Id {parent_pid} -ErrorAction SilentlyContinue
  $process = Start-Process -FilePath $installer -ArgumentList @('/VERYSILENT','/SUPPRESSMSGBOXES','/NORESTART') -PassThru -Wait
  Start-Process -FilePath $application
}} finally {{
  Remove-Item -LiteralPath $installer -Force -ErrorAction SilentlyContinue
  Remove-Item -LiteralPath $PSCommandPath -Force -ErrorAction SilentlyContinue
}}
"#
    );
    if let Err(error) = script_file
        .write_all(script.as_bytes())
        .and_then(|()| script_file.flush())
    {
        drop(script_file);
        let _ = fs::remove_file(&script_path);
        return Err(error).context("failed to prepare the update installer");
    }
    drop(script_file);

    match Command::new("powershell.exe")
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-WindowStyle",
            "Hidden",
            "-ExecutionPolicy",
            "Bypass",
            "-File",
        ])
        .arg(&script_path)
        .creation_flags(0x0800_0000)
        .spawn()
    {
        Ok(_) => Ok(()),
        Err(error) => {
            let _ = fs::remove_file(script_path);
            Err(error).context("failed to start the update installer")
        }
    }
}

#[cfg(target_os = "windows")]
fn encode_path(path: &Path) -> Result<String> {
    use std::os::windows::ffi::OsStrExt;
    let path: Vec<u16> = path.as_os_str().encode_wide().collect();
    let path = String::from_utf16(&path).context("update path is not valid UTF-16")?;
    Ok(base64_encode(path.as_bytes()))
}

#[cfg(target_os = "windows")]
fn base64_encode(input: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut output = String::with_capacity(input.len().div_ceil(3) * 4);
    for chunk in input.chunks(3) {
        let first = chunk[0];
        let second = chunk.get(1).copied().unwrap_or(0);
        let third = chunk.get(2).copied().unwrap_or(0);
        output.push(char::from(TABLE[usize::from(first >> 2)]));
        output.push(char::from(
            TABLE[usize::from(((first & 0x03) << 4) | (second >> 4))],
        ));
        output.push(if chunk.len() > 1 {
            char::from(TABLE[usize::from(((second & 0x0f) << 2) | (third >> 6))])
        } else {
            '='
        });
        output.push(if chunk.len() > 2 {
            char::from(TABLE[usize::from(third & 0x3f)])
        } else {
            '='
        });
    }
    output
}

#[cfg(test)]
mod tests {
    use super::{parse_digest, parse_version};

    #[test]
    fn compares_three_component_release_versions() {
        assert!(parse_version("0.1.3").unwrap() > parse_version("0.1.2").unwrap());
        assert!(parse_version("1.0.0").unwrap() > parse_version("0.99.99").unwrap());
        assert_eq!(parse_version("0.1.2").unwrap(), [0, 1, 2]);
    }

    #[test]
    fn rejects_malformed_versions_and_digests() {
        assert!(parse_version("0.1").is_err());
        assert!(parse_version("v0.1.2").is_err());
        assert!(parse_digest(None).is_err());
        assert!(parse_digest(Some("sha256:xyz")).is_err());
    }

    #[test]
    fn parses_github_sha256_digest() {
        let digest = format!("sha256:{}", "ab".repeat(32));
        assert_eq!(parse_digest(Some(&digest)).unwrap(), [0xab; 32]);
    }
}
