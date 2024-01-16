include!(concat!(env!("OUT_DIR"), "/decontact.rs"));

fn main() {
        let out_dir = std::env::var("OUT_DIR").unwrap_or("Default Value".to_string());
        println!("OUT_DIR is: {}", out_dir);
        println!("Hello, World2!");
}
