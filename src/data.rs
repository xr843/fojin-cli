use anyhow::{anyhow, Context, Result};
use std::io::Read;
use std::path::{Path, PathBuf};

pub struct DataSource<'a> {
    pub url: &'a str,
    pub sha256: &'a str,
}

pub fn resolve_data_path(data_dir: Option<PathBuf>) -> Result<PathBuf> {
    if let Some(d) = data_dir {
        return Ok(d.join("data.sqlite"));
    }
    let dirs = directories::ProjectDirs::from("app", "fojin", "fojin")
        .ok_or_else(|| anyhow!("无法确定缓存目录"))?;
    Ok(dirs.cache_dir().join("data.sqlite"))
}

pub fn verify_sha256(bytes: &[u8], expected_hex: &str) -> bool {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(bytes);
    let got: String = h.finalize().iter().map(|b| format!("{:02x}", b)).collect();
    got.eq_ignore_ascii_case(expected_hex)
}

pub fn gunzip(bytes: &[u8]) -> Result<Vec<u8>> {
    let mut d = flate2::read::GzDecoder::new(bytes);
    let mut out = Vec::new();
    d.read_to_end(&mut out).context("解压 gzip 失败")?;
    Ok(out)
}

pub fn ensure_data(path: &Path, offline: bool, source: &DataSource) -> Result<()> {
    if path.exists() {
        return Ok(());
    }
    if offline {
        return Err(anyhow!(
            "本地数据不存在且处于 --offline (offline)。请手动下载:\n  {}\n解压后放到: {}",
            source.url,
            path.display()
        ));
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).context("创建缓存目录失败")?;
    }
    let gz = http_get(source.url)?;
    if !verify_sha256(&gz, source.sha256) {
        return Err(anyhow!("下载校验失败(sha256 不符)。请重试或手动下载: {}", source.url));
    }
    let raw = gunzip(&gz)?;
    std::fs::write(path, raw).context("写入数据文件失败")?;
    Ok(())
}

fn http_get(url: &str) -> Result<Vec<u8>> {
    let resp = ureq::get(url).call().with_context(|| format!("下载失败: {}", url))?;
    let mut buf = Vec::new();
    resp.into_reader().read_to_end(&mut buf).context("读取响应失败")?;
    Ok(buf)
}

pub fn open_db(path: &Path) -> Result<rusqlite::Connection> {
    rusqlite::Connection::open(path).with_context(|| format!("打开数据失败: {}", path.display()))
}
