//! The MCP gateway on Home (filled in below).

use anyhow::Result;

use crate::App;
use crate::home_link::{ByteStream, HttpReply};

#[allow(clippy::too_many_arguments)]
pub async fn gateway(_app: &App, _runner_id: &str, _lease_id: &str, _server: &str, _method: &str, _headers: Vec<(String, String)>, _body: ByteStream) -> Result<HttpReply> {
    anyhow::bail!("no MCP server")
}
