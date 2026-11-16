//! The `launchpad_oracle` helper, shipped inside the binary (`oracle/launchpad_oracle/__init__.py`) and built into
//! a wheel at runtime for `uv run --with <wheel>` on Home, for evaluations and tests. Its version carries the source's hash, so uv never reuses an older build from its cache.

use std::path::{Path, PathBuf};

use anyhow::Result;
use base64::Engine;
use sha2::{Digest, Sha256};

/// The helper's source.
pub const SOURCE: &str = include_str!("../oracle/launchpad_oracle/__init__.py");
/// The helper's API version (`__version__`); the wheel adds the source's hash.
const VERSION: &str = "1.0.0";
/// The HTTP client the helper builds `ctx.http` with.
const REQUIRES: &str = "httpx2>=2.14,<3";

/// The wheel's version: `1.0.0+<hash>`.
pub fn version() -> String {
    format!("{VERSION}+{}", &hex::encode(Sha256::digest(SOURCE.as_bytes()))[..12])
}

/// The wheel's file name.
pub fn file_name() -> String {
    format!("launchpad_oracle-{}-py3-none-any.whl", version())
}

/// The wheel: a zip (stored, fixed dates) with the package, `METADATA`, `WHEEL` and `RECORD`.
pub fn wheel() -> Vec<u8> {
    let info = format!("launchpad_oracle-{}.dist-info", version());
    let mut files: Vec<(String, Vec<u8>)> = vec![
        ("launchpad_oracle/__init__.py".into(), SOURCE.as_bytes().to_vec()),
        (format!("{info}/METADATA"), format!("Metadata-Version: 2.1\nName: launchpad-oracle\nVersion: {}\nSummary: The helper of Launchpad oracles\nRequires-Python: >=3.10\nRequires-Dist: {REQUIRES}\n", version()).into_bytes()),
        (format!("{info}/WHEEL"), b"Wheel-Version: 1.0\nGenerator: launchpad\nRoot-Is-Purelib: true\nTag: py3-none-any\n".to_vec()),
    ];
    let mut record = String::new();
    for (name, data) in &files {
        let hash = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(Sha256::digest(data));
        record.push_str(&format!("{name},sha256={hash},{}\n", data.len()));
    }
    record.push_str(&format!("{info}/RECORD,,\n"));
    files.push((format!("{info}/RECORD"), record.into_bytes()));
    zip(&files)
}

/// Writes the wheel into `dir` once (an existing file of this version stays) and returns its path.
pub fn write_wheel(dir: &Path) -> Result<PathBuf> {
    let path = dir.join(file_name());
    if path.is_file() {
        return Ok(path);
    }
    std::fs::create_dir_all(dir)?;
    let tmp = dir.join(format!(".{}.{}", file_name(), uuid::Uuid::new_v4()));
    std::fs::write(&tmp, wheel())?;
    std::fs::rename(&tmp, &path)?;
    Ok(path)
}

fn crc32(data: &[u8]) -> u32 {
    let mut crc = !0u32;
    for &b in data {
        crc ^= b as u32;
        for _ in 0..8 {
            crc = if crc & 1 != 0 { (crc >> 1) ^ 0xEDB8_8320 } else { crc >> 1 };
        }
    }
    !crc
}

/// A zip archive of stored (uncompressed) files dated 1980-01-01.
fn zip(files: &[(String, Vec<u8>)]) -> Vec<u8> {
    const DATE: u16 = (1 << 5) | 1;
    let (mut out, mut central) = (Vec::new(), Vec::new());
    let le16 = |v: &mut Vec<u8>, x: u16| v.extend_from_slice(&x.to_le_bytes());
    let le32 = |v: &mut Vec<u8>, x: u32| v.extend_from_slice(&x.to_le_bytes());
    for (name, data) in files {
        let (crc, size, offset) = (crc32(data), data.len() as u32, out.len() as u32);
        le32(&mut out, 0x0403_4b50);
        for x in [20, 0, 0, 0, DATE] {
            le16(&mut out, x);
        }
        for x in [crc, size, size] {
            le32(&mut out, x);
        }
        le16(&mut out, name.len() as u16);
        le16(&mut out, 0);
        out.extend_from_slice(name.as_bytes());
        out.extend_from_slice(data);
        le32(&mut central, 0x0201_4b50);
        for x in [0x0314, 20, 0, 0, 0, DATE] {
            le16(&mut central, x);
        }
        for x in [crc, size, size] {
            le32(&mut central, x);
        }
        for x in [name.len() as u16, 0, 0, 0, 0] {
            le16(&mut central, x);
        }
        le32(&mut central, 0o100644 << 16);
        le32(&mut central, offset);
        central.extend_from_slice(name.as_bytes());
    }
    let (cd_offset, cd_size) = (out.len() as u32, central.len() as u32);
    out.extend_from_slice(&central);
    le32(&mut out, 0x0605_4b50);
    for x in [0, 0, files.len() as u16, files.len() as u16] {
        le16(&mut out, x);
    }
    le32(&mut out, cd_size);
    le32(&mut out, cd_offset);
    le16(&mut out, 0);
    out
}

#[cfg(test)]
mod tests {
    #[test]
    fn crc32_matches_the_reference() {
        assert_eq!(super::crc32(b"123456789"), 0xCBF4_3926);
    }
}
