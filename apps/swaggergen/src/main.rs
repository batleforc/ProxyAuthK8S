use api::gen_openapi;

fn main() {
    let api = gen_openapi();
    println!("{}", api.to_pretty_json().unwrap());
}
