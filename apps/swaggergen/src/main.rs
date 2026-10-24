use api::gen_openapi;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let api = gen_openapi();
    println!("{}", api.to_pretty_json()?);
    Ok(())
}
