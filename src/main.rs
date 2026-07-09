fn main() {
    match fojin_cli::run() {
        Ok(code) => std::process::exit(code),
        Err(e) => {
            eprintln!("错误: {:#}", e);
            std::process::exit(1);
        }
    }
}
