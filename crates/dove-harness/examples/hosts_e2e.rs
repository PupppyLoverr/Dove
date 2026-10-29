//! End-to-end check of sync and reconnecting sessions against a real host:
//! `cargo run -p dove-harness --example hosts_e2e -- <ssh alias> sync <repo>`
//! or `… -- <ssh alias> attach <cmd…>` (stdin/stdout are the agent's).

use dove_harness::host::Host;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    // The remote copy of this binary is started as `dove node …`.
    if args.first().is_some_and(|a| a == "node") {
        std::process::exit(dove_harness::node::main(&args[1..]));
    }
    let host = Host::Ssh { alias: args[0].clone() };
    match args[1].as_str() {
        "sync" => {
            let local = std::path::Path::new(&args[2]);
            let dir = host.remote_dir(local);
            println!("push: {:?}", dove_harness::sync::push(&host, local, &dir));
        }
        "pull" => {
            let local = std::path::Path::new(&args[2]);
            let dir = host.remote_dir(local);
            println!("pull: {:?}", dove_harness::sync::pull(&host, local, &dir));
        }
        "attach" => {
            if !dove_harness::node::remote::ensure(&host) {
                eprintln!("could not install dove on the host");
                std::process::exit(3);
            }
            let code = dove_harness::node::remote::run_attached(
                &host,
                ".",
                &args[2],
                &args[3..],
                std::time::Duration::from_secs(60),
            );
            std::process::exit(code);
        }
        other => panic!("unknown step {other}"),
    }
}
