use std::env;
use std::path::PathBuf;

use vrules_address_indexer::{apply_artifact_native, build_snapshot, write_artifact, PatchRequest};

fn main() {
    if let Err(err) = run() {
        eprintln!("vrules-address-indexer: {err}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = env::args().skip(1);
    let Some(cmd) = args.next() else {
        usage();
        return Err("missing command".into());
    };
    let opts = parse_options(args.collect())?;
    match cmd.as_str() {
        "build" => {
            let input = required_path(&opts, "--input")?;
            let out = required_path(&opts, "--out")?;
            let source = required(&opts, "--source")?;
            let generation = required(&opts, "--generation")?.parse::<u64>()?;
            let artifact = build_snapshot(&input, source, generation)?;
            write_artifact(&artifact, &out)?;
            println!(
                "built {} upserts into {}",
                artifact.manifest.upserts,
                out.display()
            );
        }
        "patch" => {
            let request = PatchRequest {
                base_dir: required_path(&opts, "--base")?,
                input_csv: required_path(&opts, "--input")?,
                source: required(&opts, "--source")?.to_string(),
                out_dir: required_path(&opts, "--out")?,
                generation: required(&opts, "--generation")?.parse::<u64>()?,
            };
            let artifact = vrules_address_indexer::build_patch(&request)?;
            write_artifact(&artifact, &request.out_dir)?;
            println!(
                "built patch: {} upserts, {} deletes into {}",
                artifact.manifest.upserts,
                artifact.manifest.deletes,
                request.out_dir.display()
            );
        }
        "install-native" => {
            let artifact = required_path(&opts, "--artifact")?;
            let db = required_path(&opts, "--db")?;
            let applied = apply_artifact_native(&artifact, &db)?;
            println!(
                "applied {} upserts and {} deletes into {}",
                applied.upserts,
                applied.deletes,
                db.display()
            );
        }
        _ => {
            usage();
            return Err(format!("unknown command `{cmd}`").into());
        }
    }
    Ok(())
}

fn usage() {
    eprintln!(
        "usage:
  vrules-address-indexer build --input us.csv --source us/xx/city --out dist/us-addresses-v1 --generation 1
  vrules-address-indexer patch --base dist/us-addresses-v1 --input us.csv --source us/xx/city --out dist/us-addresses-v2-patch --generation 2
  vrules-address-indexer install-native --artifact dist/us-addresses-v1 --db .local/us-addresses"
    );
}

fn parse_options(args: Vec<String>) -> Result<Vec<(String, String)>, Box<dyn std::error::Error>> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < args.len() {
        let key = args[i].clone();
        if !key.starts_with("--") {
            return Err(format!("expected option, got `{key}`").into());
        }
        let Some(value) = args.get(i + 1) else {
            return Err(format!("missing value for `{key}`").into());
        };
        out.push((key, value.clone()));
        i += 2;
    }
    Ok(out)
}

fn required<'a>(
    opts: &'a [(String, String)],
    key: &str,
) -> Result<&'a str, Box<dyn std::error::Error>> {
    opts.iter()
        .find(|(k, _)| k == key)
        .map(|(_, v)| v.as_str())
        .ok_or_else(|| format!("missing {key}").into())
}

fn required_path(
    opts: &[(String, String)],
    key: &str,
) -> Result<PathBuf, Box<dyn std::error::Error>> {
    Ok(PathBuf::from(required(opts, key)?))
}
