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
            let path = data::resolve_data_path(data_dir)?;
            data::ensure_data(
                &path,
                offline,
                &data::DataSource {
                    url: DATA_URL,
                    sha256: DATA_SHA256,
                },
            )?;
            let conn = data::open_db(&path)?;
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
    }
}
