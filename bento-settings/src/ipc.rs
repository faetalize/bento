//! Questions for the running compositor.

use bento_ipc::socket::Socket;
use bento_ipc::{Get, Request, Response};
use niri_ipc::LayoutEngine;

fn request(request: Request) -> Result<Response, String> {
    let mut socket = Socket::connect().map_err(|err| format!("Couldn't reach bento: {err}"))?;
    match socket.send(&request) {
        Ok(reply) => reply,
        Err(err) => Err(format!("Couldn't talk to bento: {err}")),
    }
}

pub fn layout_engine() -> Result<LayoutEngine, String> {
    match request(Request::Get { what: Get::Engine })? {
        Response::Engine(engine) => Ok(engine),
        _ => Err(String::from("bento sent an unexpected reply")),
    }
}

pub fn version() -> Result<String, String> {
    match request(Request::Get { what: Get::Version })? {
        Response::Version(version) => Ok(version),
        _ => Err(String::from("bento sent an unexpected reply")),
    }
}
