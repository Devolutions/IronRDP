use std::time::Instant;

pub struct Section {
    name: &'static str,
    start: Instant,
}

impl Section {
    pub fn new(name: &'static str) -> Section {
        flush_all();
        eprintln!("::group::{name}");
        let start = Instant::now();
        Self { name, start }
    }
}

impl Drop for Section {
    fn drop(&mut self) {
        flush_all();
        eprintln!("{}: {:.2?}", self.name, self.start.elapsed());
        eprintln!("::endgroup::");
    }
}

fn flush_all() {
    use std::io::{self, Write as _};

    if let Err(err) = io::stdout().flush() {
        eprintln!("stdout flush error: {err}");
    }
    if let Err(err) = io::stderr().flush() {
        eprintln!("stderr flush error: {err}");
    }
}
