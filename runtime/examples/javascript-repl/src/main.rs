fn main() {
    if let Err(error) = javascript_repl::run() {
        eprintln!("error: {error}");
        std::process::exit(1);
    }
}
