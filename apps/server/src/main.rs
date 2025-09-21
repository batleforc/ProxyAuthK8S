use actix_cors::Cors;
use actix_web::{
    dev::Service, get, http::header, middleware::Compress, post, web, App, HttpResponse,
    HttpServer, Responder, Scope,
};
use api::api_doc::ApiDoc;
use trace::{shutdown_tracing, start_tracing};
use tracing::{error, info, instrument};
use tracing_actix_web::{RequestId, TracingLogger};
use utoipa::OpenApi;
use utoipa_scalar::{Scalar, Servable};

#[instrument]
#[get("/")]
async fn hello() -> impl Responder {
    info!("Someone called the hello endpoint");
    HttpResponse::Ok().body("Hello world!")
}

#[post("/echo")]
async fn echo(req_body: String) -> impl Responder {
    HttpResponse::Ok().body(req_body)
}

#[instrument(level = "error")]
async fn manual_hello() -> impl Responder {
    error!("Saying hey there!");
    HttpResponse::Ok().body("Hey there!")
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    println!(include_str!("banner.art"));
    let (logger, trace, meter) = start_tracing(&trace::Context {
        pod_name: std::env::var("POD_NAME").unwrap_or_else(|_| "not_a_pod".to_string()),
    });

    let mut api_doc = ApiDoc::openapi();
    api_doc.info.version = env!("CARGO_PKG_VERSION").to_string();
    let server = HttpServer::new(move || {
        let cors = Cors::default()
            .allow_any_origin()
            .allow_any_method()
            .allow_any_header()
            .max_age(3600);
        App::new()
            .wrap(cors)
            .wrap(Compress::default())
            .service(Scalar::with_url("/api/docs", api_doc.clone()))
            .service(
                Scope::new("/api")
                    .wrap_fn(|mut req, srv| {
                        let request_id_asc = req.extract::<RequestId>();
                        let fut = srv.call(req);
                        async move {
                            let mut res = fut.await?;
                            let request_id: RequestId = request_id_asc.await.unwrap();
                            let request_id_str = format!("{}", request_id);
                            let headers = res.headers_mut();
                            headers.insert(
                                header::HeaderName::from_static("x-request-id"),
                                header::HeaderValue::from_str(request_id_str.as_str()).unwrap(),
                            );
                            Ok(res)
                        }
                    })
                    .wrap(TracingLogger::default())
                    .route("/hey", web::get().to(manual_hello))
                    .service(hello)
                    .service(echo),
            )
    })
    .bind(("0.0.0.0", 8080))?
    .shutdown_timeout(5);

    tokio::join!(server.run()).0?;
    if let Err(e) = shutdown_tracing(logger, trace, meter) {
        eprintln!("Error during the shutdown of tracing: {e}");
    }
    Ok(())
}
