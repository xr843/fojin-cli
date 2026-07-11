use anyhow::Result;
use clap::{Parser, Subcommand};
use rusqlite::Connection;
use std::io::Read;
use std::path::PathBuf;

use crate::{data, normalize, query, render};

/// Release process sets DATA_SHA256 to the published artifact's checksum.
pub const DATA_URL: &str =
    "https://github.com/xr843/fojin-cli/releases/download/data-v1/fojin-parallels-v1.sqlite.gz";
pub const DATA_SHA256: &str = "e9a203a9f4021fca880e997b26aae134814f1ab34ce3f284a963b7320211fa7f";

#[derive(Parser)]
#[command(name = "fojin", version, about = "fojin 跨藏对读 CLI(离线 · 无需登录)")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand)]
pub enum Command {
    /// 查询一段汉文的跨语平行(梵/巴/藏)
    Parallel {
        /// 汉文查询串;省略时从 stdin 读取
        query: Option<String>,
        /// 只看某些语种,逗号分隔,如 sa,bo,pi
        #[arg(long)]
        lang: Option<String>,
        /// 每语最多 N 条
        #[arg(
            long,
            default_value_t = 3,
            value_parser = clap::builder::RangedU64ValueParser::<usize>::new().range(1..)
        )]
        top: usize,
        /// 最多显示 N 组匹配
        #[arg(
            long,
            default_value_t = 10,
            value_parser = clap::builder::RangedU64ValueParser::<usize>::new().range(1..)
        )]
        limit: usize,
        /// 显示全部匹配组,忽略 --limit
        #[arg(long)]
        all: bool,
        /// 机器可读 JSON 输出
        #[arg(long)]
        json: bool,
        /// 指定数据目录(覆盖默认缓存)
        #[arg(long)]
        data_dir: Option<PathBuf>,
        /// 不联网;缺数据则报错
        #[arg(long)]
        offline: bool,
    },
    /// 本地数据管理:状态 / 校验 / 清理 / 重新下载
    Data {
        #[command(subcommand)]
        action: DataAction,
    },
    /// 按经名关键词搜索收录文本(简繁均可)
    Texts {
        /// 经名关键词,如 "心经"
        keyword: String,
        /// 机器可读 JSON 输出
        #[arg(long)]
        json: bool,
        /// 指定数据目录(覆盖默认缓存)
        #[arg(long)]
        data_dir: Option<PathBuf>,
        /// 不联网;缺数据则报错
        #[arg(long)]
        offline: bool,
    },
    /// 按 Taishō 编号列出某经的对齐(经文顺序,非相关度排序)
    Cite {
        /// Taishō 编号,如 T0251(大小写不敏感)
        cbeta_id: String,
        /// 只看某一卷
        #[arg(long)]
        juan: Option<i64>,
        /// 只看某些语种,逗号分隔,如 sa,bo
        #[arg(long)]
        lang: Option<String>,
        /// 每语最多 N 条
        #[arg(
            long,
            default_value_t = 3,
            value_parser = clap::builder::RangedU64ValueParser::<usize>::new().range(1..)
        )]
        top: usize,
        /// 最多显示 N 组
        #[arg(
            long,
            default_value_t = 10,
            value_parser = clap::builder::RangedU64ValueParser::<usize>::new().range(1..)
        )]
        limit: usize,
        /// 显示全部匹配组,忽略 --limit
        #[arg(long)]
        all: bool,
        /// 机器可读 JSON 输出
        #[arg(long)]
        json: bool,
        /// 指定数据目录(覆盖默认缓存)
        #[arg(long)]
        data_dir: Option<PathBuf>,
        /// 不联网;缺数据则报错
        #[arg(long)]
        offline: bool,
    },
}

#[derive(Subcommand)]
pub enum DataAction {
    /// 显示本地数据状态(不触发下载)
    Status {
        /// 机器可读 JSON 输出
        #[arg(long)]
        json: bool,
        /// 指定数据目录(覆盖默认缓存)
        #[arg(long)]
        data_dir: Option<PathBuf>,
    },
    /// 删除本地数据,释放空间(下次在线运行会重新下载)
    Clean {
        /// 指定数据目录(覆盖默认缓存)
        #[arg(long)]
        data_dir: Option<PathBuf>,
    },
    /// 重新下载数据,覆盖本地副本
    Update {
        /// 指定数据目录(覆盖默认缓存)
        #[arg(long)]
        data_dir: Option<PathBuf>,
    },
    /// 校验本地数据版本 / 规范 / SQLite 与 FTS 完整性(不触发下载)
    Verify {
        /// 机器可读 JSON 输出
        #[arg(long)]
        json: bool,
        /// 指定数据目录(覆盖默认缓存)
        #[arg(long)]
        data_dir: Option<PathBuf>,
    },
}

pub fn compute_output(
    conn: &Connection,
    raw: &str,
    langs: Option<&[String]>,
    top: usize,
    limit: Option<usize>,
    json: bool,
) -> Result<String> {
    let map = normalize::load_norm_map(conn)?;
    let norm = normalize::normalize(raw.trim(), &map);
    normalize::validate_query_length(&norm)?;
    let groups_all = query::search(conn, &norm, langs, top)?;
    let total = groups_all.len();
    let shown = match limit {
        Some(n) => n.min(total),
        None => total,
    };
    let shown_groups = &groups_all[..shown];
    let hidden = total - shown;
    Ok(if json {
        render::render_json(shown_groups, total)
    } else {
        render::render_human(shown_groups, langs, hidden)
    })
}

pub fn run() -> Result<i32> {
    let cli = Cli::parse();
    match cli.command {
        Command::Parallel {
            query,
            lang,
            top,
            limit: limit_flag,
            all,
            json,
            data_dir,
            offline,
        } => {
            let raw = match query {
                Some(q) => q,
                None => {
                    let mut s = String::new();
                    std::io::stdin().read_to_string(&mut s)?;
                    s
                }
            };
            if raw.trim().is_empty() {
                eprintln!("用法: fojin parallel \"色即是空\"  (或管道: echo ... | fojin parallel)");
                return Ok(2);
            }
            let preflight = normalize::normalize(raw.trim(), &normalize::NormMap::new());
            normalize::validate_query_length(&preflight)?;
            let conn = open_ensured(data_dir, offline)?;
            let langs: Option<Vec<String>> = lang.map(|l| {
                l.split(',')
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect()
            });
            let limit = if all { None } else { Some(limit_flag) };
            let out = compute_output(&conn, &raw, langs.as_deref(), top, limit, json)?;
            println!("{out}");
            Ok(0)
        }
        Command::Data { action } => run_data(action),
        Command::Texts {
            keyword,
            json,
            data_dir,
            offline,
        } => {
            if keyword.trim().is_empty() {
                eprintln!("用法: fojin texts \"心经\"");
                return Ok(2);
            }
            let conn = open_ensured(data_dir, offline)?;
            let map = normalize::load_norm_map(&conn)?;
            let norm_kw = normalize::normalize(keyword.trim(), &map);
            let hits = query::texts_matching(&conn, &norm_kw, &map)?;
            if json {
                let v = serde_json::json!({ "total": hits.len(), "texts": hits });
                println!("{}", serde_json::to_string_pretty(&v)?);
            } else {
                println!("{}", render::render_texts(&hits));
            }
            Ok(0)
        }
        Command::Cite {
            cbeta_id,
            juan,
            lang,
            top,
            limit: limit_flag,
            all,
            json,
            data_dir,
            offline,
        } => {
            let conn = open_ensured(data_dir, offline)?;
            let langs: Option<Vec<String>> = lang.map(|l| {
                l.split(',')
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect()
            });
            let groups_all = query::by_cbeta(&conn, cbeta_id.trim(), juan, langs.as_deref(), top)?;
            let total = groups_all.len();
            if total == 0 && !json {
                let juan_hint = juan.map(|j| format!(" 卷{j}")).unwrap_or_default();
                println!(
                    "未找到 {}{} 的对齐。可用 fojin texts <经名关键词> 查询收录文本。",
                    cbeta_id.trim(),
                    juan_hint
                );
                return Ok(0);
            }
            let shown = if all { total } else { limit_flag.min(total) };
            let hidden = total - shown;
            let out = if json {
                render::render_json(&groups_all[..shown], total)
            } else {
                render::render_human(&groups_all[..shown], langs.as_deref(), hidden)
            };
            println!("{out}");
            Ok(0)
        }
    }
}

/// Resolve the data path, ensure the dataset is present (downloading unless
/// offline), and open it.
fn open_ensured(data_dir: Option<PathBuf>, offline: bool) -> Result<Connection> {
    let path = data::resolve_data_path(data_dir)?;
    data::ensure_data(
        &path,
        offline,
        &data::DataSource {
            url: DATA_URL,
            sha256: DATA_SHA256,
        },
    )?;
    data::open_compatible_db(&path)
}

fn run_data(action: DataAction) -> Result<i32> {
    const MB: u64 = 1024 * 1024;
    match action {
        DataAction::Status { json, data_dir } => {
            let path = data::resolve_data_path(data_dir)?;
            let exists = path.exists();
            let size_bytes = if exists {
                Some(std::fs::metadata(&path)?.len())
            } else {
                None
            };
            let stats = if exists {
                Some(data::dataset_stats(&data::open_db(&path)?)?)
            } else {
                None
            };
            if json {
                let by_lang: serde_json::Map<String, serde_json::Value> = stats
                    .as_ref()
                    .map(|s| {
                        s.by_lang
                            .iter()
                            .map(|(l, c)| (l.clone(), serde_json::json!(c)))
                            .collect()
                    })
                    .unwrap_or_default();
                let v = serde_json::json!({
                    "path": path.display().to_string(),
                    "exists": exists,
                    "size_bytes": size_bytes,
                    "version": stats.as_ref().and_then(|s| s.version.clone()),
                    "license": stats.as_ref().and_then(|s| s.license.clone()),
                    "attribution": stats.as_ref().and_then(|s| s.attribution.clone()),
                    "total": stats.as_ref().map(|s| s.total),
                    "by_lang": by_lang,
                    "texts": stats.as_ref().map(|s| s.texts),
                });
                println!("{}", serde_json::to_string_pretty(&v)?);
            } else {
                println!(
                    "{}",
                    render::render_status(&path.display().to_string(), size_bytes, stats.as_ref())
                );
            }
            Ok(0)
        }
        DataAction::Clean { data_dir } => {
            let path = data::resolve_data_path(data_dir)?;
            match data::clean_data(&path)? {
                Some(size) => {
                    println!("已删除 {} (释放 {} MB)", path.display(), size / MB);
                }
                None => {
                    println!("本地无数据,无需清理: {}", path.display());
                }
            }
            Ok(0)
        }
        DataAction::Update { data_dir } => {
            let path = data::resolve_data_path(data_dir)?;
            data::update_data(
                &path,
                &data::DataSource {
                    url: DATA_URL,
                    sha256: DATA_SHA256,
                },
            )?;
            println!("数据已更新: {}", path.display());
            Ok(0)
        }
        DataAction::Verify { json, data_dir } => {
            let path = data::resolve_data_path(data_dir)?;
            if !path.exists() {
                anyhow::bail!(
                    "本地数据不存在: {}。请先运行 `fojin data update`。",
                    path.display()
                );
            }
            let compatibility = data::verify_dataset_file(&path)?;
            if json {
                let v = serde_json::json!({
                    "ok": true,
                    "version": compatibility.version,
                    "norm_ruleset": compatibility.norm_ruleset,
                });
                println!("{}", serde_json::to_string_pretty(&v)?);
            } else {
                println!(
                    "数据校验通过 version={} norm_ruleset={}",
                    compatibility.version, compatibility.norm_ruleset
                );
            }
            Ok(0)
        }
    }
}
