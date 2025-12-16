use clap::Parser;
use std::fs;
use std::path::PathBuf;

/// SBE XML to Rust code generator.
#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// Input SBE XML schema file
    #[arg(short = 'i', long)]
    input: PathBuf,
    /// Output directory for generated Rust modules
    #[arg(short = 'o', long)]
    output: PathBuf,
    /// Endianness for multi‑byte fields (little or big)
    #[arg(short = 'e', long, default_value = "little", value_parser = ["little", "big"])]
    endian: String,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();
    let xml = fs::read_to_string(&args.input)?;
    let opts = sbe_gen::GeneratorOptions {
        endian: args.endian.clone(),
    };
    sbe_gen::generate_to(&xml, &args.output, &opts)?;
    Ok(())
}
