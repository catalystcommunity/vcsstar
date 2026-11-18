fn main() {
    // Rebuild the embedded migration list when SQL files change or are added.
    println!("cargo:rerun-if-changed=../../migrations");
}
