fn main() {
    #[cfg(windows)]
    {
        let mut res = winres::WindowsResource::new();
        res.set_icon("icon.ico");
        res.set("ProductName", "MiniUnicorn");
        res.set("FileDescription", "MiniUnicorn v0.2.3");
        if let Err(e) = res.compile() {
            eprintln!("winres failed: {e}");
            std::process::exit(1);
        }
    }
}
