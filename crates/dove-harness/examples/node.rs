//! `dove node …` without the app: `cargo run -p dove-harness --example node
//! -- node attach s1 -- cat`. Lets the session daemon be tested by hand.

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let rest = if args.first().is_some_and(|a| a == "node") { &args[1..] } else { &args[..] };
    std::process::exit(dove_harness::node::main(rest));
}
