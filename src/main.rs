fn main() {
    if let Err(error) = deff::run() {
        eprintln!("deff failed: {error}");
        std::process::exit(1);
    }
}
