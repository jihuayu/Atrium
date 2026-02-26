pub mod d1;
pub mod http;
pub mod routes;

use worker::{event, Context, Env, Request, Response, Result};

#[event(fetch)]
pub async fn fetch(req: Request, env: Env, _ctx: Context) -> Result<Response> {
    let state = routes::WorkerState::from_env(&env);
    routes::router(state).run(req, env).await
}
