fn main() {
    pkg_config::Config::new()
        .atleast_version("0.70")
        .probe("vte-2.91")
        .expect("Falta libvte-2.91-dev");
}
