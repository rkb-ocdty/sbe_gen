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
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();
    let xml = fs::read_to_string(&args.input)?;
    let opts = sbe_gen::GeneratorOptions::default();
    sbe_gen::generate_to(&xml, &args.output, &opts)?;
    Ok(())
}
