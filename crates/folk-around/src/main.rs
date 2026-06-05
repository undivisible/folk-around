use std::process;
use std::sync::Arc;

use folk_computer_use::register_tools;
use folk_core::{AccessMode, AppConfig, generate_pairing_code, load_config, save_config};
use folk_mcp::ToolTable;
use folk_transport::{run_http, run_stdio, start_p2p};

const VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Debug, Default)]
struct Cli {
    verbose: bool,
    mode: Option<String>,
    http_port: Option<u16>,
    signal_url: Option<String>,
    room: Option<String>,
    p2p_requested: bool,
    stdio_requested: bool,
    help: bool,
    version: bool,
}

fn main() {
    if let Err(err) = run() {
        eprintln!("{err}");
        process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let cli = parse_args();
    if cli.help {
        print_help();
        return Ok(());
    }
    if cli.version {
        println!("folk-around v{VERSION}");
        return Ok(());
    }

    let saved = load_config()?;
    let mut signal_url = cli.signal_url;
    let mut http_port = cli.http_port;
    let mode_name = cli
        .mode
        .or(saved.mode.clone())
        .unwrap_or_else(|| "full".to_string());

    if cli.stdio_requested {
        signal_url = None;
        http_port = None;
    } else {
        if cli.p2p_requested && signal_url.is_none() {
            signal_url = saved
                .signal_url
                .clone()
                .or_else(|| Some("https://folkaround.undivisible.dev".to_string()));
        }
        if http_port.is_none() {
            http_port = saved.http_port;
        }
    }

    let Some(mode) = AccessMode::parse(&mode_name) else {
        eprintln!("invalid mode. use: full, limited, sandbox");
        process::exit(1);
    };

    let mut table = ToolTable::new(mode);
    register_tools(&mut table);
    let table = Arc::new(table);

    if let Some(url) = signal_url {
        let room = cli
            .room
            .or(saved.room)
            .unwrap_or_else(generate_pairing_code);
        let port = http_port.unwrap_or(8080);
        save_config(&AppConfig {
            signal_url: Some(url.clone()),
            room: Some(room.clone()),
            http_port: Some(port),
            mode: Some(mode_name),
        })?;
        if cli.verbose {
            eprintln!("[folk] P2P mode, signaling: {url}");
        }
        start_p2p(cli.verbose, Arc::clone(&table), url.clone(), room.clone());
        print_pairing_instructions(&url, &room, port);
        run_http(cli.verbose, table, port)?;
    } else if let Some(port) = http_port {
        save_config(&AppConfig {
            signal_url: saved.signal_url,
            room: saved.room,
            http_port: Some(port),
            mode: Some(mode_name),
        })?;
        if cli.verbose {
            eprintln!("[folk] HTTP SSE mode on port {port}");
        } else {
            eprintln!("[folk] HTTP listening on http://127.0.0.1:{port}/");
        }
        run_http(cli.verbose, table, port)?;
    } else {
        if cli.verbose {
            eprintln!("[folk] stdio mode (mode={})", mode.as_str());
        }
        run_stdio(cli.verbose, table)?;
    }
    Ok(())
}

fn parse_args() -> Cli {
    parse_args_from(std::env::args().skip(1))
}

fn parse_args_from<I, S>(args: I) -> Cli
where
    I: IntoIterator<Item = S>,
    S: Into<String>,
{
    let mut cli = Cli::default();
    let args = args.into_iter().map(Into::into).collect::<Vec<_>>();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--verbose" | "-v" => cli.verbose = true,
            "--mode" => {
                i += 1;
                if let Some(value) = args.get(i) {
                    cli.mode = Some(value.clone());
                }
            }
            "--http" => {
                i += 1;
                if let Some(value) = args.get(i) {
                    cli.http_port = value.parse::<u16>().ok();
                }
            }
            "--signal-server" => {
                i += 1;
                if let Some(value) = args.get(i) {
                    cli.signal_url = Some(value.clone());
                }
            }
            "--room" => {
                i += 1;
                if let Some(value) = args.get(i) {
                    cli.room = Some(value.clone());
                }
            }
            "--p2p" => cli.p2p_requested = true,
            "--stdio" => cli.stdio_requested = true,
            "--help" | "-h" => cli.help = true,
            "--version" | "-V" => cli.version = true,
            _ => {}
        }
        i += 1;
    }
    cli
}

fn print_pairing_instructions(signal_url: &str, room: &str, port: u16) {
    eprintln!("[folk] pairing code: {room}");
    eprintln!("[folk] give this code to the client and use signaling server: {signal_url}");
    eprintln!("[folk] local MCP endpoint: http://127.0.0.1:{port}/sse");
    eprintln!("[folk] waiting for peer...");
}

fn print_help() {
    eprintln!(
        r#"folk-around v{VERSION} - MCP computer use daemon
Usage: folk-around [options]

  --verbose           Show tool calls
  --mode <mode>       full, limited, sandbox (default: full)
  --stdio             Force stdio transport and ignore saved transport
  --http <port>       HTTP SSE transport (e.g. --http 8080)
  --p2p               Join saved/default signaling server and expose local HTTP
  --signal-server <url>  Custom signaling server URL
  --room <name>       Pairing code / room name
  --version           Print version
  --help              This help

Transports:
  no args   Reuses saved HTTP port, or stdio if none is saved
  --stdio   Standard MCP over stdin/stdout
  --http    HTTP SSE for remote over Tailscale/SSH
  --p2p     Prints a pairing code, registers with signaling, and starts local MCP
"#
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_flag_should_parse() {
        let cli = parse_args_from(["--version"]);

        assert!(cli.version);
        assert!(!cli.help);
    }

    #[test]
    fn short_version_flag_should_parse() {
        let cli = parse_args_from(["-V"]);

        assert!(cli.version);
    }
}
