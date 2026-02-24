//! Roomie — ultralight networked game server.
//!
//! One task per connection; Tokio handles many thousands of concurrent streams cheaply.
//! See DESIGN.md for architecture.

use roomie::server;

#[tokio::main]
async fn main() -> std::io::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let tcp_bind = args
        .get(1)
        .map(String::as_str)
        .unwrap_or("0.0.0.0:8765");
    // Default: TCP + WebSocket + HTTP stats. One arg = TCP only; two = TCP + WS; three = TCP + WS + HTTP.
    let ws_bind = match (args.get(1), args.get(2)) {
        (None, _) => Some("0.0.0.0:8766"),
        (_, Some(ws)) => Some(ws.as_str()),
        (_, None) => None,
    };
    let http_bind = match (args.get(1), args.get(2), args.get(3)) {
        (None, _, _) => Some("0.0.0.0:80"),
        (_, _, Some(http)) => Some(http.as_str()),
        _ => None,
    };
    server::run(tcp_bind, ws_bind, http_bind).await
}
