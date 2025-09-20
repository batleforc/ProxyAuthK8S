use api::api_doc::ApiDoc;
use utoipa::OpenApi;
fn main() {
    println!("{}", ApiDoc::openapi().to_pretty_json().unwrap());
}
