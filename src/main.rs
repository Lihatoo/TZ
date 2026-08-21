use clap::Parser;
use tz::cli::Cli; // cli 当前是由 lib.rs 管理的,它属于库 crate tz，不是 main.rs 所属二进制 crate 的直接模块。

fn main() {
    let cli: Cli = Cli::parse();
    if let Err(error) = tz::cli::run(cli) {
        eprintln!("{error}"); // Err    → 不是函数，也不是普通变量
        std::process::exit(1); // error  → 变量名
    }
}
