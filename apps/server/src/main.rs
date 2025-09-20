use actix_web::{get, post, web, App, HttpResponse, HttpServer, Responder};
use api::api_doc::ApiDoc;
use utoipa::OpenApi;
use utoipa_scalar::{Scalar, Servable};

#[get("/")]
async fn hello() -> impl Responder {
    HttpResponse::Ok().body("Hello world!")
}

#[post("/echo")]
async fn echo(req_body: String) -> impl Responder {
    HttpResponse::Ok().body(req_body)
}

async fn manual_hello() -> impl Responder {
    HttpResponse::Ok().body("Hey there!")
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    println!(include_str!("banner.art"));

    let mut api_doc = ApiDoc::openapi();
    api_doc.info.version = env!("CARGO_PKG_VERSION").to_string();
    let server = HttpServer::new(move || {
        App::new()
            .service(hello)
            .service(echo)
            .service(Scalar::with_url("/api/docs", api_doc.clone()))
            .route("/hey", web::get().to(manual_hello))
    })
    .bind(("0.0.0.0", 8080))?
    .shutdown_timeout(5);

    tokio::join!(server.run()).0?;
    Ok(())
}
