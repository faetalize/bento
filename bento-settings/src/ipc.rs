//! Questions for the running compositor.

use niri_ipc::socket::Socket;
use niri_ipc::{LayoutEngine, Request, Response};

fn request(request: Request) -> Result<Response, String> {
    let mut socket = Socket::connect().map_err(|err| format!("Couldn't reach bento: {err}"))?;
    match socket.send(request) {
        Ok(Ok(response)) => Ok(response),
        Ok(Err(message)) => Err(message),
        Err(err) => Err(format!("Couldn't talk to bento: {err}")),
    }
}

pub fn layout_engine() -> Result<LayoutEngine, String> {
    match request(Request::LayoutEngine)? {
        Response::LayoutEngine(engine) => Ok(engine),
        _ => Err(String::from("bento sent an unexpected reply")),
    }
}

pub fn version() -> Result<String, String> {
    match request(Request::Version)? {
        Response::Version(version) => Ok(version),
        _ => Err(String::from("bento sent an unexpected reply")),
    }
}
