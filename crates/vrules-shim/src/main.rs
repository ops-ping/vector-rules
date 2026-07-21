use std::net::SocketAddr;
use std::path::PathBuf;

use vrules_shim::{
    ComponentManifest, ComponentOutput, DaemonConfig, ManifestPath, RuntimeHost, run_daemon,
    run_stdio,
};

#[derive(Debug)]
enum Mode {
    Stdio,
    Daemon(SocketAddr),
}

#[derive(Debug)]
struct Args {
    mode: Mode,
    manifest: ManifestPath,
    embedding_model: Option<PathBuf>,
    embedding_model_name: Option<String>,
}

impl Args {
    fn parse() -> Result<Self, String> {
        let mut mode = Mode::Stdio;
        let mut manifest = None;
        let mut embedding_model = None;
        let mut embedding_model_name = None;
        let mut args = std::env::args().skip(1);
        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--daemon" => mode = Mode::Daemon("127.0.0.1:8765".parse().unwrap()),
                "--bind" => {
                    let value = args.next().ok_or("--bind requires an address")?;
                    let address = value
                        .parse()
                        .map_err(|e| format!("invalid --bind address `{value}`: {e}"))?;
                    mode = Mode::Daemon(address);
                }
                "--manifest" => {
                    manifest = Some(PathBuf::from(
                        args.next().ok_or("--manifest requires a path")?,
                    ));
                }
                "--embedding-model" => {
                    embedding_model = Some(PathBuf::from(
                        args.next()
                            .ok_or("--embedding-model requires a GGUF path")?,
                    ));
                }
                "--embedding-model-name" => {
                    embedding_model_name = Some(
                        args.next()
                            .ok_or("--embedding-model-name requires a value")?,
                    );
                }
                "--help" | "-h" => {
                    println!(
                        "usage: vrules-shim [--manifest PATH] [--embedding-model GGUF [--embedding-model-name NAME]] [--daemon [--bind ADDRESS]]\n\
                         default: MCP over stdio\n\
                         --embedding-model: use a local GGUF embedding model instead of the manifest default\n\
                         --daemon: admin PWA and HTTP/WebSocket transports"
                    );
                    std::process::exit(0);
                }
                other => return Err(format!("unknown argument `{other}`")),
            }
        }
        if embedding_model_name.is_some() && embedding_model.is_none() {
            return Err("--embedding-model-name requires --embedding-model".to_string());
        }
        Ok(Self {
            mode,
            manifest: ManifestPath::resolve(manifest)?,
            embedding_model,
            embedding_model_name,
        })
    }
}

fn main() -> anyhow::Result<()> {
    let args = Args::parse().map_err(anyhow::Error::msg)?;
    let mut manifest = ComponentManifest::load(&args.manifest)?;
    if let Some(model) = args.embedding_model {
        manifest.use_embedding_model(&model, args.embedding_model_name.as_deref())?;
    }
    let output = match args.mode {
        Mode::Stdio => ComponentOutput::Stdio,
        Mode::Daemon(_) => ComponentOutput::Daemon,
    };
    let upstream = std::env::var("VRULES_REST_UPSTREAM")
        .ok()
        .filter(|value| !value.trim().is_empty());
    let host = RuntimeHost::load(manifest, output, upstream)?;
    match args.mode {
        Mode::Stdio => run_stdio(host),
        Mode::Daemon(bind) => tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()?
            .block_on(run_daemon(host, DaemonConfig { bind })),
    }
}
