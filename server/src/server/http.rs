//! HTTP server (axum) that serves anonymized JSON stats: games (id, rooms, players).

use axum::{extract::State, routing::get, Json, Router};

use super::AppRegistry;

/// Anonymized per-game stats for the public JSON endpoint.
#[derive(serde::Serialize)]
struct GameStats {
    id: String,
    rooms: u32,
    players: u32,
}

#[derive(serde::Serialize)]
struct StatsResponse {
    games: Vec<GameStats>,
}

fn collect_stats(registry: &AppRegistry) -> StatsResponse {
    let games = registry
        .iter()
        .map(|entry| {
            let (rooms, players) = entry.value().room_and_player_counts();
            GameStats {
                id: entry.key().clone(),
                rooms: rooms as u32,
                players: players as u32,
            }
        })
        .collect();
    StatsResponse { games }
}

async fn stats_handler(State(registry): State<AppRegistry>) -> Json<StatsResponse> {
    Json(collect_stats(&registry))
}

/// Serve HTTP on `bind`. GET / returns JSON `{"games": [{"id":"pong", "rooms": 5, "players": 10}, ...]}`.
pub async fn run_http(bind: &str, registry: AppRegistry) -> std::io::Result<()> {
    let app = Router::new()
        .route("/", get(stats_handler))
        .with_state(registry);

    let listener = tokio::net::TcpListener::bind(bind).await?;
    eprintln!("roomie HTTP (stats) listening on {}", bind);

    axum::serve(listener, app).await.map_err(|e| {
        std::io::Error::new(std::io::ErrorKind::Other, e)
    })
}
