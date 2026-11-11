use clap::Parser;
use ditherr_cli::{run, Cli};

fn main() {
    let cli = Cli::parse();
    if let Err(e) = run(cli, &mut std::io::stdout()) {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}
