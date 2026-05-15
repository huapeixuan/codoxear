use codoxear_backend_rs::broker::{config, shim};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let cli = match config::parse_cli(&args) {
        Ok(cli) => cli,
        Err(message) => {
            eprintln!("codoxear-broker-rs: {message}");
            std::process::exit(2);
        }
    };
    shim::exec_python_bridge(cli);
}
