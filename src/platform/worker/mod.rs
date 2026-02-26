pub mod d1;
pub mod http;
pub mod routes;

use worker::{event, Context, Env, Method, Request, Response, Result};

#[event(fetch)]
pub async fn fetch(req: Request, env: Env, _ctx: Context) -> Result<Response> {
    if req.method() == Method::Options {
        return add_cors(Response::empty()?.with_status(204));
    }
    let state = routes::WorkerState::from_env(&env);
    let response = routes::router(state).run(req, env).await?;
    add_cors(response)
}

fn add_cors(mut response: Response) -> Result<Response> {
    let h = response.headers_mut();
    h.set("Access-Control-Allow-Origin", "*")?;
    h.set("Access-Control-Allow-Methods", "GET, POST, PATCH, DELETE, OPTIONS")?;
    h.set("Access-Control-Allow-Headers", "Authorization, Content-Type, Accept")?;
    h.set("Access-Control-Expose-Headers", "Link")?;
    Ok(response)
}
