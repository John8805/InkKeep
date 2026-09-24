//! inkkeep-core 的開發用 CLI。
//!
//! 密碼從 `INKKEEP_PASSWORD` 環境變數讀，沒有就從 stdin 讀一行。
//! 不做終端機遮蔽輸入。

use chrono::Local;
use clap::{Parser, Subcommand, ValueEnum};
use inkkeep_core::crypto;
use inkkeep_core::gen::{self, GenOpts};
use inkkeep_core::import;
use inkkeep_core::model::{self, Item, ItemKind};
use inkkeep_core::search::{self, Index};
use inkkeep_core::template::{ItemResolver, RenderCtx, Template};
use inkkeep_core::vault::{conflict_files, Vault};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::process::ExitCode;

#[derive(Parser)]
#[command(name = "inkkeep", about = "inkkeep 核心的開發用 CLI")]
struct Cli {
    /// kdbx 保險庫路徑
    #[arg(long, global = true, default_value = "vault.kdbx")]
    vault: PathBuf,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// 建立新的保險庫
    Create,
    /// 印出第一層金鑰，可以直接貼進 KeePassXC
    VaultKey,
    /// 新增一筆
    Add {
        #[arg(long, value_enum, default_value_t = Kind::Snippet)]
        kind: Kind,
        #[arg(long)]
        title: String,
        #[arg(long)]
        body: String,
        /// 只有 password 類型會用到
        #[arg(long)]
        user: Option<String>,
        #[arg(long = "tag")]
        tags: Vec<String>,
    },
    /// 列出全部，順序同空查詢
    List,
    /// 搜尋
    Search { query: Vec<String> },
    /// 顯示一筆的完整內容
    Show { id: String },
    /// 展開一筆片語的模板
    Render {
        id: String,
        /// 形式為 標籤=值，可重複
        #[arg(long = "input")]
        inputs: Vec<String>,
        /// 模擬剪貼簿內容
        #[arg(long)]
        clipboard: Option<String>,
        /// 用預覽模式，不需要提供輸入值
        #[arg(long)]
        preview: bool,
    },
    /// 刪除一筆
    Delete { id: String },
    /// 記一次使用
    Touch { id: String },
    /// 產生密碼，不需要保險庫
    Gen {
        #[arg(long, default_value_t = 20)]
        length: u8,
        #[arg(long)]
        no_symbols: bool,
        #[arg(long)]
        no_digits: bool,
        #[arg(long)]
        allow_lookalike: bool,
        /// 產生幾組
        #[arg(long, default_value_t = 1)]
        count: u8,
    },
    /// 顯示保險庫狀態
    Info,
    /// 讀瀏覽器書籤，只報數量不寫入
    ImportPreview {
        #[arg(long, value_enum, default_value_t = BrowserSource::Chrome)]
        source: BrowserSource,
        /// 自訂書籤檔路徑
        #[arg(long)]
        path: Option<PathBuf>,
    },
    /// 匯入瀏覽器書籤
    Import {
        #[arg(long, value_enum, default_value_t = BrowserSource::Chrome)]
        source: BrowserSource,
        #[arg(long)]
        path: Option<PathBuf>,
    },
}

#[derive(Clone, Copy, ValueEnum)]
enum BrowserSource {
    Chrome,
    Edge,
    Firefox,
    File,
}

impl From<BrowserSource> for import::Source {
    fn from(b: BrowserSource) -> Self {
        match b {
            BrowserSource::Chrome => import::Source::Chrome,
            BrowserSource::Edge => import::Source::Edge,
            BrowserSource::Firefox => import::Source::Firefox,
            BrowserSource::File => import::Source::File,
        }
    }
}

#[derive(Clone, Copy, ValueEnum)]
enum Kind {
    Snippet,
    Bookmark,
    Password,
}

impl From<Kind> for ItemKind {
    fn from(k: Kind) -> Self {
        match k {
            Kind::Snippet => ItemKind::Snippet,
            Kind::Bookmark => ItemKind::Bookmark,
            Kind::Password => ItemKind::Password,
        }
    }
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("錯誤：{e}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();

    if let Command::ImportPreview { source, path } = &cli.command {
        let bookmarks = import::read((*source).into(), path.as_deref())?;
        println!("讀到 {} 筆書籤", bookmarks.len());
        for bm in bookmarks.iter().take(10) {
            let tags = if bm.folders.is_empty() {
                String::new()
            } else {
                format!("  [{}]", bm.folders.join("/"))
            };
            println!(
                "  {}{tags}
    {}",
                bm.title, bm.url
            );
        }
        if bookmarks.len() > 10 {
            println!("  …還有 {} 筆", bookmarks.len() - 10);
        }
        return Ok(());
    }

    if let Command::Gen {
        length,
        no_symbols,
        no_digits,
        allow_lookalike,
        count,
    } = cli.command
    {
        let opts = GenOpts {
            length,
            symbols: !no_symbols,
            digits: !no_digits,
            exclude_lookalike: !allow_lookalike,
            ..GenOpts::default()
        };
        for _ in 0..count.max(1) {
            println!("{}", gen::generate_os(&opts)?);
        }
        eprintln!("熵 {:.0} bits", opts.entropy_bits());
        return Ok(());
    }

    let password = read_password()?;

    if matches!(cli.command, Command::Create) {
        let vault = Vault::create(&cli.vault, &password)?;
        println!("已建立 {}", vault.path().display());
        return Ok(());
    }

    let mut vault = Vault::open_with_password(&cli.vault, &password)?;

    match cli.command {
        Command::Create | Command::Gen { .. } | Command::ImportPreview { .. } => {
            unreachable!("前面已處理")
        }

        Command::Import { source, path } => {
            let bookmarks = import::read(source.into(), path.as_deref())?;
            let (to_add, report) = import::merge(vault.items(), bookmarks);
            if !to_add.is_empty() {
                vault.upsert_many(to_add)?;
            }
            println!("新增 {} 筆，跳過 {} 筆重複", report.added, report.skipped);
        }

        Command::VaultKey => {
            println!("{}", vault.vault_key());
        }

        Command::Add {
            kind,
            title,
            body,
            user,
            tags,
        } => {
            let mut item = Item::new(kind.into(), title, body);
            item.username = user;
            item.tags = tags;
            if let Err(errs) = model::validate(&item) {
                for e in &errs {
                    eprintln!("驗證失敗：{e}");
                }
                return Err("項目未通過驗證".into());
            }
            if item.kind.uses_template() {
                Template::parse(&item.body)?;
            }
            if item.kind == ItemKind::Password && !item.body.is_empty() {
                let public = vault
                    .public_key()
                    .ok_or("這個保險庫還沒有公鑰，先用 app 解鎖一次")?;
                item.body = crypto::seal(&public, &item.body)?;
            }
            let id = item.id;
            vault.upsert(item)?;
            println!("{id}");
        }

        Command::List => print_list(&vault, vault_order(&vault, "")),

        Command::Search { query } => {
            let q = query.join(" ");
            print_list(&vault, vault_order(&vault, &q));
        }

        Command::Show { id } => {
            let item = find(&vault, &id)?;
            println!("id       {}", item.id);
            println!("類型     {:?}", item.kind);
            println!("標題     {}", item.title);
            if let Some(u) = &item.username {
                println!("帳號     {u}");
            }
            println!("標籤     {}", item.tags.join(", "));
            println!("使用次數 {}", item.use_count);
            println!("---");
            if item.kind == ItemKind::Password && !item.body.is_empty() {
                let secret = vault.unlock_secrets(&password)?;
                println!("{}", crypto::unseal(&secret, &item.body)?);
            } else {
                println!("{}", item.body);
            }
        }

        Command::Render {
            id,
            inputs,
            clipboard,
            preview,
        } => {
            let item = find(&vault, &id)?.clone();
            if !item.kind.uses_template() {
                println!("{}", item.body);
                return Ok(());
            }
            let template = Template::parse(&item.body)?;
            let resolver = VaultResolver(vault.items().to_vec());

            let mut map = BTreeMap::new();
            for pair in &inputs {
                let (k, v) = pair
                    .split_once('=')
                    .ok_or_else(|| format!("--input 要寫成 標籤=值，拿到 {pair:?}"))?;
                map.insert(k.to_string(), v.to_string());
            }

            let plan = template.plan(&resolver)?;
            if !preview {
                for field in &plan.needs_inputs {
                    if !map.contains_key(field.label()) {
                        eprintln!("提示：缺少輸入 {:?}", field.label());
                    }
                }
            }

            let mut ctx = RenderCtx::new(Local::now()).with_inputs(map);
            if let Some(c) = clipboard {
                ctx = ctx.with_clipboard(c);
            }
            if preview {
                ctx = ctx.preview();
            }

            let rendered = template.render(&ctx, &resolver)?;
            println!("{}", rendered.text);
            if rendered.cursor_from_end > 0 {
                eprintln!("游標距文末 {} 個字元", rendered.cursor_from_end);
            }
        }

        Command::Delete { id } => {
            let item = find(&vault, &id)?.clone();
            vault.delete(item.id)?;
            println!("已刪除 {}", item.title);
        }

        Command::Touch { id } => {
            let item = find(&vault, &id)?.clone();
            vault.touch(item.id)?;
            println!("{} 使用次數 {}", item.title, item.use_count + 1);
        }

        Command::Info => {
            println!("路徑     {}", vault.path().display());
            println!("筆數     {}", vault.items().len());
            let conflicts = conflict_files(vault.path());
            if conflicts.is_empty() {
                println!("衝突副本 無");
            } else {
                println!("衝突副本 {} 個：", conflicts.len());
                for c in conflicts {
                    println!("  {}", c.display());
                }
            }
        }
    }

    Ok(())
}

/// `#標籤` 形式的 token 當標籤過濾，其餘當搜尋文字。
fn vault_order(vault: &Vault, query: &str) -> Vec<usize> {
    let index = Index::build(vault.items());
    let (tags, terms): (Vec<&str>, Vec<&str>) = query
        .split_whitespace()
        .partition(|t| t.len() > 1 && t.starts_with('#'));
    let tags: Vec<String> = tags.iter().map(|t| t[1..].to_string()).collect();
    search::search(
        &index,
        vault.items(),
        &terms.join(" "),
        &tags,
        None,
        None,
        search::DEFAULT_LIMIT,
    )
}

fn print_list(vault: &Vault, idxs: Vec<usize>) {
    for i in idxs {
        let item = &vault.items()[i];
        let short: String = item.id.simple().to_string().chars().take(8).collect();
        let kind = match item.kind {
            ItemKind::Snippet => "片語",
            ItemKind::Bookmark => "書籤",
            ItemKind::Password => "密碼",
        };
        let excerpt: String = match item.kind {
            ItemKind::Password => item.username.clone().unwrap_or_default(),
            _ => item.body.replace('\n', "⏎").chars().take(40).collect(),
        };
        println!(
            "{short}  {kind}  {:<20} {:>4}  {excerpt}",
            item.title, item.use_count
        );
    }
}

/// 用 id 前綴找。
fn find<'a>(vault: &'a Vault, prefix: &str) -> Result<&'a Item, Box<dyn std::error::Error>> {
    let matches: Vec<&Item> = vault
        .items()
        .iter()
        .filter(|i| i.id.simple().to_string().starts_with(prefix))
        .collect();
    match matches.len() {
        0 => Err(format!("找不到 id 開頭是 {prefix:?} 的項目").into()),
        1 => Ok(matches[0]),
        n => Err(format!("{prefix:?} 對到 {n} 筆，請給更長的前綴").into()),
    }
}

struct VaultResolver(Vec<Item>);

impl ItemResolver for VaultResolver {
    fn resolve(&self, title: &str) -> Option<String> {
        let key = title.to_lowercase();
        self.0
            .iter()
            .filter(|i| i.kind.uses_template() && i.title.to_lowercase() == key)
            .max_by_key(|i| i.updated_at)
            .map(|i| i.body.clone())
    }
}

fn read_password() -> Result<String, Box<dyn std::error::Error>> {
    if let Ok(p) = std::env::var("INKKEEP_PASSWORD") {
        return Ok(p);
    }
    eprint!("主密碼：");
    let mut line = String::new();
    std::io::stdin().read_line(&mut line)?;
    let p = line.trim_end_matches(['\r', '\n']).to_string();
    if p.is_empty() {
        return Err("沒有輸入密碼".into());
    }
    Ok(p)
}
