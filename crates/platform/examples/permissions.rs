//! Prints what the OS allows this process. `cargo run -p wf-platform --example permissions`

fn main() {
    println!("{:?}", wf_platform::permissions::check());
}
