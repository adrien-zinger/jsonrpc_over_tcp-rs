use clap::{Parser, Subcommand};
use ljsonrpc_over_tcp::*;
use std::{fs, process, sync::Arc};

#[derive(Parser)]
#[command(version, about, long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    Run {
        #[arg(long)]
        addr: String,
        #[arg(long)]
        path: String,
    },
    Call {
        #[arg(long)]
        addr: String,
        #[arg(long)]
        method: String,
        #[arg(long)]
        params: String,
    },
}

/** Run jsonrpc process by calling registred script.
*   Return process standard output in case there are.
*/
fn exec_cmd(script: &str, input: String) -> std::io::Result<Option<String>> {
    let mut cmd = process::Command::new(script);
    println!("exec script {} with args: {:?}", script, input);
    cmd.args([input]);
    if let Ok(output) = cmd.output() {
        println!("command received produced {}: {:#?}", script, output);
        if let Ok(res) = String::from_utf8(output.stdout) {
            Ok(Some(res))
        } else {
            Ok(Some("error server".into()))
        }
    } else {
        Ok(Some("error server".into()))
    }
}

fn main() {
    let cli = Cli::parse();
    match cli.command {
        Commands::Run { addr, path } => {
            println!("Starting server on address '{addr}'");
            let mut server = Server::new(addr);
            let files = fs::read_dir(path).expect("Unable to read given path");
            for file in files {
                let file_path = file.unwrap().path();
                let file_name = file_path.file_name().unwrap().to_str().unwrap();
                let script = file_path.to_str().unwrap().to_string().clone();
                println!("Register {} method", file_name);
                server.add_method(file_name, Arc::new(move |params| exec_cmd(&script, params)));
            }
            server.run().unwrap();
            server.block_on();
        }

        Commands::Call {
            addr,
            method,
            params,
        } => println!("result: {}", send(&addr, "1", &method, &params).unwrap()),
    }
}
