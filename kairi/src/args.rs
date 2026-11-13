use std::path::PathBuf;

use clap::Parser;

#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
pub struct Args {
    #[arg(short, long)]
    pub config: Option<String>,

    #[arg(short, long)]
    pub test: bool,

    #[arg(short, long)]
    pub debug: bool,

    /// State directory, the mailing lists are mirrored under `<DATA_DIR>/public-inbox`
    #[arg(long, default_value = "/var/lib/kairi")]
    pub data_dir: PathBuf,
}
