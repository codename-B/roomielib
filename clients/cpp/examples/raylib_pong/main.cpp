/**
 * Roomie Pong — Raylib example.
 * Run roomie server first, then run two instances of this.
 * One: Create room, share the 6-char code. Other: Join with that code.
 * Host: Start game (countdown 3-2-1), then play. Ball is host-authoritative; clients send paddle + ball_reflected (JSON).
 */
#include "roomie/roomie_client.hpp"
#include "raylib.h"
#include "nlohmann/json.hpp"
#include <algorithm>
#include <cmath>
#include <cstring>
#include <string>
#include <vector>

using json = nlohmann::json;

static const int SCREEN_W = 800;
static const int SCREEN_H = 600;
static const float MAP_W = 800.f;
static const float MAP_H = 600.f;
static const float PADDLE_W = 12.f;
static const float PADDLE_H = 80.f;
static const float BALL_R = 8.f;
static const float PADDLE_SPEED = 400.f;
static const float BALL_SPEED = 350.f;
static const char* APP_KEY = "pong";
static const char* SERVER_HOST = "server.roomielib.com";
static const char* SERVER_PORT = "8765";

enum class GameScreen {
    Menu,        // Connect + Create or Join or Browse
    BrowsingRooms, // Requested room list; show list and pick one to join
    LobbyHost,   // Created room, show code, Start button
    LobbyGuest,  // Joined, waiting for host
    Countdown,   // 3, 2, 1
    Playing,
    Error
};

struct GameState {
    GameScreen screen = GameScreen::Menu;
    roomie::RoomieClient client;
    std::string room_code;
    std::string join_code_input;
    std::string error_msg;
    std::string client_hash;

    // Lobby / menu
    bool creating = false;
    bool joining = false;
    bool entering_join_code = false;  // true after clicking "Join room" (so we only capture key input for code)
    std::string lobby_reject_msg;     // e.g. "minimum 2 players required to start" when MoveRejected
    std::vector<roomie::RoomListEntry> room_list;  // filled when browsing rooms
    int room_list_scroll = 0;        // scroll offset for browse list

    // Countdown
    int countdown_num = 3;
    double countdown_timer = 0.0;

    // Playing state (from JSON PongState)
    float ball_x = MAP_W / 2.f, ball_y = MAP_H / 2.f;
    float ball_vx = 0.f, ball_vy = 0.f;
    float paddle0_y = (MAP_H - PADDLE_H) / 2.f;
    float paddle1_y = (MAP_H - PADDLE_H) / 2.f;
    int score0 = 0, score1 = 0;
    bool game_started = false;
    int remote_countdown = 0;

    // My paddle; which side (0 or 1) is server-assigned when we join (first available paddle)
    int my_side = 0;
    float my_paddle_y = (MAP_H - PADDLE_H) / 2.f;
    bool ball_reflected_this_frame = false;

    // Input send throttle (don't spam)
    double last_input_sent = 0.0;

    // Window focus (for resync when guest regains focus after being in background)
    bool window_focused = true;
};

static std::string make_client_hash() {
    return "pong_" + std::to_string(GetRandomValue(1000, 9999));
}

// --- JSON state (host sends) ---
static std::string state_to_json(GameState& g) {
    json j;
    j["ball"] = {{"pos", {{"x", g.ball_x}, {"y", g.ball_y}}}, {"vel", {{"x", g.ball_vx}, {"y", g.ball_vy}}}};
    j["paddles"] = {{{"side", 0}, {"y", g.paddle0_y}}, {{"side", 1}, {"y", g.paddle1_y}}};
    j["scores"] = {g.score0, g.score1};
    j["map_width"] = MAP_W;
    j["map_height"] = MAP_H;
    j["countdown"] = g.remote_countdown;
    j["game_started"] = g.game_started;
    std::string s = j.dump();
    return s;
}

static void state_from_json(GameState& g, const std::string& str) {
    try {
        json j = json::parse(str);
        if (j.contains("ball")) {
            g.ball_x = j["ball"]["pos"]["x"].get<float>();
            g.ball_y = j["ball"]["pos"]["y"].get<float>();
            g.ball_vx = j["ball"]["vel"]["x"].get<float>();
            g.ball_vy = j["ball"]["vel"]["y"].get<float>();
        }
        if (j.contains("paddles") && j["paddles"].size() >= 2) {
            g.paddle0_y = j["paddles"][0]["y"].get<float>();
            g.paddle1_y = j["paddles"][1]["y"].get<float>();
        }
        if (j.contains("scores") && j["scores"].size() >= 2) {
            g.score0 = j["scores"][0].get<int>();
            g.score1 = j["scores"][1].get<int>();
        }
        if (j.contains("countdown")) g.remote_countdown = j["countdown"].get<int>();
        if (j.contains("game_started")) g.game_started = j["game_started"].get<bool>();
    } catch (...) {}
}

// --- JSON input (clients send to host) ---
static std::string input_to_json(int side, float paddle_y, bool ball_reflected) {
    json j;
    j["paddle_y"] = paddle_y;
    j["ball_reflected"] = ball_reflected;
    j["side"] = side;
    return j.dump();
}

static void input_from_json(uint32_t peer_id, GameState& g, const uint8_t* data, size_t len, uint32_t host_peer_id) {
    try {
        std::string str(data, data + len);
        json j = json::parse(str);
        int side = j.value("side", 1);
        float y = j.value("paddle_y", (MAP_H - PADDLE_H) / 2.f);
        bool reflected = j.value("ball_reflected", false);
        if (side == 0) g.paddle0_y = y; else g.paddle1_y = y;
        if (reflected) {
            if (side == 0) g.ball_vx = std::abs(g.ball_vx);
            else g.ball_vx = -std::abs(g.ball_vx);
        }
    } catch (...) {}
}

int main() {
    GameState g;
    g.client_hash = make_client_hash();

    SetConfigFlags(FLAG_VSYNC_HINT);
    InitWindow(SCREEN_W, SCREEN_H, "Roomie Pong");
    SetTargetFPS(60);

    roomie::ClientCallbacks cbs;
    cbs.on_joined = [&g](const std::string& room_code, uint32_t peer_id, uint32_t host_peer_id,
                         uint8_t side, const roomie::RoomSettings&, const uint8_t* initial_state, size_t state_len) {
        g.room_code = room_code;
        g.my_side = (side <= 1) ? (int)side : 0;
        if (peer_id == host_peer_id) {
            g.screen = GameScreen::LobbyHost;
            g.client.set_room_public(true);  // so others can find this room via Browse
        } else {
            g.screen = GameScreen::LobbyGuest;
        }
        if (initial_state && state_len > 0) {
            std::string s(initial_state, initial_state + state_len);
            state_from_json(g, s);
            // Sync our paddle from the state we just received
            g.my_paddle_y = (g.my_side == 0) ? g.paddle0_y : g.paddle1_y;
            // Join in progress: game already started — go straight to Countdown or Playing
            if (g.game_started) {
                if (g.remote_countdown > 0) {
                    g.screen = GameScreen::Countdown;
                    g.countdown_num = g.remote_countdown;
                    g.countdown_timer = 0.0;
                } else {
                    g.screen = GameScreen::Playing;
                }
            }
        }
    };
    cbs.on_peer_joined = [](uint32_t, const std::string&, const std::string*) {};
    cbs.on_peer_left = [](uint32_t) {};
    cbs.on_host_transferred = [&g](uint32_t new_host_peer_id) {
        // We became the new host (old host disconnected) — request current state so we can take over
        if (new_host_peer_id == g.client.peer_id())
            g.client.request_state();
    };
    cbs.on_peer_input = [&g](uint32_t peer_id, const uint8_t* data, size_t len) {
        if (!g.client.is_host() || g.screen != GameScreen::Playing) return;
        input_from_json(peer_id, g, data, len, g.client.host_peer_id());
    };
    cbs.on_state_update = [&g](const uint8_t* data, size_t len) {
        std::string s(data, data + len);
        state_from_json(g, s);
        // Keep guest's paddle in sync with host-authoritative state (host is authority for their own paddle)
        if (!g.client.is_host() && g.client.peer_id() != 0)
            g.my_paddle_y = (g.my_side == 0) ? g.paddle0_y : g.paddle1_y;
        if (g.screen == GameScreen::LobbyGuest && g.game_started && g.remote_countdown > 0) {
            g.screen = GameScreen::Countdown;
            g.countdown_num = g.remote_countdown;
            g.countdown_timer = 0.0;
        }
        if (g.screen == GameScreen::Countdown && g.remote_countdown <= 0 && g.countdown_num <= 0)
            g.screen = GameScreen::Playing;
        if (g.screen == GameScreen::Playing && !g.game_started && g.remote_countdown <= 0)
            g.screen = GameScreen::Playing;
    };
    cbs.on_move_rejected = [&g](const std::string& reason) {
        // Server rejected our start (e.g. not enough players) — stay in lobby and show reason
        g.screen = GameScreen::LobbyHost;
        g.game_started = false;
        g.remote_countdown = 0;
        g.countdown_num = 3;
        g.lobby_reject_msg = reason;
    };
    cbs.on_error = [&g](const std::string& msg) {
        g.error_msg = msg;
        g.screen = GameScreen::Error;
    };
    cbs.on_room_list = [&g](const std::vector<roomie::RoomListEntry>& list) {
        g.room_list = list;
        g.room_list_scroll = 0;
        g.screen = GameScreen::BrowsingRooms;
    };
    g.client.set_callbacks(std::move(cbs));

    while (!WindowShouldClose()) {
        float dt = GetFrameTime();
        g.client.poll();

        // When window is unfocused, OS may throttle our loop — drain socket so we don't fall behind
        bool focused = IsWindowFocused();
        if (!focused && g.client.is_connected() && g.client.peer_id() != 0) {
            for (int i = 0; i < 20; i++) {
                g.client.poll();
            }
        }
        // When guest regains focus, request full state to resync (fixes desync after tabbing away)
        if (focused && !g.window_focused && g.client.is_connected() && !g.client.is_host() && g.client.peer_id() != 0) {
            g.client.request_state();
        }
        g.window_focused = focused;

        // --- Countdown timer (local) ---
        if (g.screen == GameScreen::Countdown) {
            g.countdown_timer += dt;
            if (g.countdown_timer >= 1.0) {
                g.countdown_timer -= 1.0;
                g.countdown_num--;
                if (g.countdown_num <= 0)
                    g.screen = GameScreen::Playing;
            }
        }

        // --- Input ---
        if (g.screen == GameScreen::Menu) {
            Vector2 mouse = GetMousePosition();
            bool click = IsMouseButtonPressed(MOUSE_LEFT_BUTTON);

            if (!g.client.is_connected()) {
                Rectangle connectBtn = { 80, 220, 200, 44 };
                if (click && CheckCollisionPointRec(mouse, connectBtn)) {
                    if (g.client.connect(SERVER_HOST, SERVER_PORT)) {}
                    else g.screen = GameScreen::Error, g.error_msg = g.client.last_error();
                }
            } else {
                if (!g.entering_join_code) {
                    Rectangle createBtn = { 80, 220, 200, 44 };
                    Rectangle joinBtn  = { 300, 220, 200, 44 };
                    Rectangle browseBtn = { 520, 220, 200, 44 };
                    if (click && CheckCollisionPointRec(mouse, createBtn)) {
                        g.client.create_room(APP_KEY, g.client_hash, nullptr);
                        g.creating = true;
                    }
                    if (click && CheckCollisionPointRec(mouse, joinBtn)) {
                        g.join_code_input.clear();
                        g.entering_join_code = true;
                    }
                    if (click && CheckCollisionPointRec(mouse, browseBtn)) {
                        g.room_list.clear();
                        g.client.list_rooms(APP_KEY);
                    }
                } else {
                    Rectangle joinSubmitBtn = { 80, 340, 160, 44 };
                    Rectangle backBtn       = { 260, 340, 120, 44 };
                    if (click && CheckCollisionPointRec(mouse, joinSubmitBtn) && !g.join_code_input.empty()) {
                        std::string code = g.join_code_input;
                        while (!code.empty() && (code.back() == ' ' || code.back() == '\t' || code.back() == '\r' || code.back() == '\n'))
                            code.pop_back();
                        size_t start = 0;
                        while (start < code.size() && (code[start] == ' ' || code[start] == '\t' || code[start] == '\r' || code[start] == '\n'))
                            start++;
                        if (start > 0) code.erase(0, start);
                        if (!code.empty()) {
                            g.client.join_room(APP_KEY, code, g.client_hash, nullptr);
                            g.joining = true;
                            g.entering_join_code = false;
                        }
                    }
                    if (click && CheckCollisionPointRec(mouse, backBtn)) {
                        g.entering_join_code = false;
                        g.join_code_input.clear();
                    }
                    int key = GetCharPressed();
                    if (key >= 32 && key < 127 && g.join_code_input.size() < 8)
                        g.join_code_input += (char)key;
                    if (IsKeyPressed(KEY_BACKSPACE) && !g.join_code_input.empty())
                        g.join_code_input.pop_back();
                }
            }
        }

        // Browsing rooms: scroll (mouse wheel), click Join or Back
        if (g.screen == GameScreen::BrowsingRooms) {
            const float list_y_start = 140.f;
            const float row_h = 44.f;
            const int visible_count = 8;
            const int total = (int)g.room_list.size();
            const int max_scroll = std::max(0, total - visible_count);
            g.room_list_scroll = std::clamp(g.room_list_scroll, 0, max_scroll);
            int wheel = (int)GetMouseWheelMove();
            if (wheel != 0)
                g.room_list_scroll = std::clamp(g.room_list_scroll - wheel, 0, max_scroll);
            Vector2 mouse = GetMousePosition();
            bool click = IsMouseButtonPressed(MOUSE_LEFT_BUTTON);
            for (int i = 0; i < visible_count; i++) {
                int idx = g.room_list_scroll + i;
                if (idx >= total) break;
                float ry = list_y_start + i * row_h;
                Rectangle joinBtn = { 500.f, ry, 100.f, row_h - 4 };
                if (click && CheckCollisionPointRec(mouse, joinBtn)) {
                    const auto& e = g.room_list[idx];
                    g.client.join_room(APP_KEY, e.room_code, g.client_hash, nullptr);
                    g.joining = true;
                    break;
                }
            }
            float back_y = list_y_start + visible_count * row_h + 20;
            Rectangle backBtn = { 80.f, back_y, 120, 44 };
            if (click && CheckCollisionPointRec(mouse, backBtn)) {
                g.screen = GameScreen::Menu;
                g.room_list.clear();
            }
        }

        if (g.screen == GameScreen::LobbyHost && IsKeyPressed(KEY_ENTER)) {
            g.lobby_reject_msg.clear();
            g.game_started = true;
            g.remote_countdown = 3;
            g.ball_x = MAP_W / 2.f;
            g.ball_y = MAP_H / 2.f;
            float angle = (float)GetRandomValue(0, 1) ? 0.4f : -0.4f;
            g.ball_vx = BALL_SPEED * cosf(angle);
            g.ball_vy = BALL_SPEED * sinf(angle);
            std::string state = state_to_json(g);
            g.client.send_move((const uint8_t*)state.data(), state.size());
            g.screen = GameScreen::Countdown;
            g.countdown_num = 3;
            g.countdown_timer = 0.0;
        }

        if (g.screen == GameScreen::Playing) {
            // Same controls for both players: W/S move your paddle (my_side is server-assigned)
            if (IsKeyDown(KEY_W)) g.my_paddle_y -= PADDLE_SPEED * dt;
            if (IsKeyDown(KEY_S)) g.my_paddle_y += PADDLE_SPEED * dt;
            g.my_paddle_y = fmaxf(0, fminf(MAP_H - PADDLE_H, g.my_paddle_y));

            if (g.client.is_host()) {
                if (g.my_side == 0) g.paddle0_y = g.my_paddle_y; else g.paddle1_y = g.my_paddle_y;
                // Ball physics (host)
                g.ball_x += g.ball_vx * dt;
                g.ball_y += g.ball_vy * dt;
                // Walls
                if (g.ball_y <= BALL_R || g.ball_y >= MAP_H - BALL_R) g.ball_vy = -g.ball_vy;
                // Paddles (simple bounce; PeerInput ball_reflected can override)
                float px0 = 20.f, px1 = MAP_W - 20.f;
                if (g.ball_vx < 0 && g.ball_x - BALL_R <= px0 + PADDLE_W && g.ball_x + BALL_R >= px0 &&
                    g.ball_y >= g.paddle0_y && g.ball_y <= g.paddle0_y + PADDLE_H) {
                    g.ball_vx = -g.ball_vx;
                    g.ball_x = px0 + PADDLE_W + BALL_R;
                }
                if (g.ball_vx > 0 && g.ball_x + BALL_R >= px1 - PADDLE_W && g.ball_x - BALL_R <= px1 &&
                    g.ball_y >= g.paddle1_y && g.ball_y <= g.paddle1_y + PADDLE_H) {
                    g.ball_vx = -g.ball_vx;
                    g.ball_x = px1 - PADDLE_W - BALL_R;
                }
                // Score
                if (g.ball_x < -BALL_R) { g.score1++; g.ball_x = MAP_W/2; g.ball_y = MAP_H/2; g.ball_vx = BALL_SPEED; g.ball_vy = 0; }
                if (g.ball_x > MAP_W + BALL_R) { g.score0++; g.ball_x = MAP_W/2; g.ball_y = MAP_H/2; g.ball_vx = -BALL_SPEED; g.ball_vy = 0; }
                std::string state = state_to_json(g);
                g.client.send_move((const uint8_t*)state.data(), state.size());
            } else {
                std::string input = input_to_json(g.my_side, g.my_paddle_y, g.ball_reflected_this_frame);
                g.client.send_input((const uint8_t*)input.data(), input.size());
                g.ball_reflected_this_frame = false;
            }
        }

        // --- Draw ---
        BeginDrawing();
        ClearBackground(BLACK);

        if (g.screen == GameScreen::Menu) {
            DrawText("Roomie Pong", 80, 80, 40, LIGHTGRAY);
            Vector2 mouse = GetMousePosition();

            if (!g.client.is_connected()) {
                Rectangle connectBtn = { 80, 220, 200, 44 };
                bool hover = CheckCollisionPointRec(mouse, connectBtn);
                DrawRectangleRec(connectBtn, hover ? LIME : DARKGRAY);
                DrawRectangleLinesEx(connectBtn, 2, hover ? WHITE : GRAY);
                DrawText("Connect", (int)(connectBtn.x + (connectBtn.width - MeasureText("Connect", 22)) / 2), (int)(connectBtn.y + 10), 22, hover ? BLACK : LIGHTGRAY);
            } else if (!g.entering_join_code) {
                DrawText("Create or join a room:", 80, 180, 20, GRAY);
                Rectangle createBtn = { 80, 220, 200, 44 };
                Rectangle joinBtn   = { 300, 220, 200, 44 };
                Rectangle browseBtn = { 520, 220, 200, 44 };
                bool hoverCreate = CheckCollisionPointRec(mouse, createBtn);
                bool hoverJoin   = CheckCollisionPointRec(mouse, joinBtn);
                bool hoverBrowse = CheckCollisionPointRec(mouse, browseBtn);
                DrawRectangleRec(createBtn, hoverCreate ? LIME : DARKGRAY);
                DrawRectangleLinesEx(createBtn, 2, hoverCreate ? WHITE : GRAY);
                DrawText("Create room", (int)(createBtn.x + (createBtn.width - MeasureText("Create room", 22)) / 2), (int)(createBtn.y + 10), 22, hoverCreate ? BLACK : LIGHTGRAY);
                DrawRectangleRec(joinBtn, hoverJoin ? LIME : DARKGRAY);
                DrawRectangleLinesEx(joinBtn, 2, hoverJoin ? WHITE : GRAY);
                DrawText("Join room", (int)(joinBtn.x + (joinBtn.width - MeasureText("Join room", 22)) / 2), (int)(joinBtn.y + 10), 22, hoverJoin ? BLACK : LIGHTGRAY);
                DrawRectangleRec(browseBtn, hoverBrowse ? LIME : DARKGRAY);
                DrawRectangleLinesEx(browseBtn, 2, hoverBrowse ? WHITE : GRAY);
                DrawText("Browse rooms", (int)(browseBtn.x + (browseBtn.width - MeasureText("Browse rooms", 22)) / 2), (int)(browseBtn.y + 10), 22, hoverBrowse ? BLACK : LIGHTGRAY);
            } else {
                DrawText("Enter room code:", 80, 200, 20, GRAY);
                DrawText(g.join_code_input.empty() ? "_" : g.join_code_input.c_str(), 80, 240, 28, WHITE);
                Rectangle joinSubmitBtn = { 80, 340, 160, 44 };
                Rectangle backBtn       = { 260, 340, 120, 44 };
                bool hoverJoin = CheckCollisionPointRec(mouse, joinSubmitBtn);
                bool hoverBack = CheckCollisionPointRec(mouse, backBtn);
                DrawRectangleRec(joinSubmitBtn, hoverJoin ? LIME : DARKGRAY);
                DrawRectangleLinesEx(joinSubmitBtn, 2, hoverJoin ? WHITE : GRAY);
                DrawText("Join", (int)(joinSubmitBtn.x + (joinSubmitBtn.width - MeasureText("Join", 22)) / 2), (int)(joinSubmitBtn.y + 10), 22, hoverJoin ? BLACK : LIGHTGRAY);
                DrawRectangleRec(backBtn, hoverBack ? DARKGRAY : GRAY);
                DrawRectangleLinesEx(backBtn, 2, hoverBack ? WHITE : LIGHTGRAY);
                DrawText("Back", (int)(backBtn.x + (backBtn.width - MeasureText("Back", 22)) / 2), (int)(backBtn.y + 10), 22, LIGHTGRAY);
            }
        } else if (g.screen == GameScreen::BrowsingRooms) {
            DrawText("Open rooms (scroll, click to join)", 80, 80, 24, LIGHTGRAY);
            Vector2 mouse = GetMousePosition();
            const float list_y_start = 140.f;
            const float row_h = 44.f;
            const int visible_count = 8;
            int total = (int)g.room_list.size();
            int scroll = std::clamp(g.room_list_scroll, 0, std::max(0, total - visible_count));
            for (int i = 0; i < visible_count; i++) {
                int idx = scroll + i;
                if (idx >= total) break;
                const auto& e = g.room_list[idx];
                float ry = list_y_start + i * row_h;
                Rectangle joinBtn = { 500.f, ry, 100.f, row_h - 4 };
                bool hover = CheckCollisionPointRec(mouse, joinBtn);
                DrawText(e.room_code.c_str(), 80, (int)ry + 8, 20, WHITE);
                DrawText(TextFormat("%u / %u", (unsigned)e.player_count, (unsigned)e.settings.max_players), 260, (int)ry + 8, 18, GRAY);
                DrawRectangleRec(joinBtn, hover ? LIME : DARKGRAY);
                DrawRectangleLinesEx(joinBtn, 2, hover ? WHITE : GRAY);
                DrawText("Join", (int)(joinBtn.x + (joinBtn.width - MeasureText("Join", 20)) / 2), (int)(joinBtn.y + 10), 20, hover ? BLACK : LIGHTGRAY);
            }
            if (total == 0) {
                DrawText("No open rooms. Create one or ask for a code.", 80, (int)list_y_start + 20, 18, GRAY);
            }
            float back_y = list_y_start + visible_count * row_h + 20;
            Rectangle backBtn = { 80.f, back_y, 120, 44 };
            bool hoverBack = CheckCollisionPointRec(mouse, backBtn);
            DrawRectangleRec(backBtn, hoverBack ? DARKGRAY : GRAY);
            DrawRectangleLinesEx(backBtn, 2, hoverBack ? WHITE : LIGHTGRAY);
            DrawText("Back", (int)(backBtn.x + (backBtn.width - MeasureText("Back", 22)) / 2), (int)(backBtn.y + 10), 22, LIGHTGRAY);
        } else if (g.screen == GameScreen::LobbyHost) {
            DrawText("Room code (share this):", 80, 120, 24, GRAY);
            DrawText(g.room_code.c_str(), 80, 160, 48, LIME);
            DrawText("Press ENTER to start game", 80, 280, 22, YELLOW);
            if (!g.lobby_reject_msg.empty()) {
                DrawText(("Cannot start: " + g.lobby_reject_msg).c_str(), 80, 320, 18, RED);
            }
        } else if (g.screen == GameScreen::LobbyGuest) {
            DrawText("Waiting for host to start...", 80, 200, 24, GRAY);
            DrawText(("Room: " + g.room_code).c_str(), 80, 260, 20, LIGHTGRAY);
        } else if (g.screen == GameScreen::Countdown) {
            const char* msg = g.countdown_num > 0 ? (g.countdown_num == 3 ? "3" : g.countdown_num == 2 ? "2" : "1") : "GO!";
            int sz = g.countdown_num > 0 ? 120 : 80;
            int x = SCREEN_W/2 - MeasureText(msg, sz)/2;
            int y = SCREEN_H/2 - sz/2;
            DrawText(msg, x, y, sz, YELLOW);
        } else if (g.screen == GameScreen::Playing) {
            float px0 = 20.f, px1 = MAP_W - 20.f;
            // Use my_paddle_y for local player (immediate feedback); state for remote
            float draw_y0 = (g.my_side == 0) ? g.my_paddle_y : g.paddle0_y;
            float draw_y1 = (g.my_side == 1) ? g.my_paddle_y : g.paddle1_y;
            DrawRectangle((int)px0, (int)draw_y0, (int)PADDLE_W, (int)PADDLE_H, WHITE);
            DrawRectangle((int)(px1 - PADDLE_W), (int)draw_y1, (int)PADDLE_W, (int)PADDLE_H, WHITE);
            DrawCircle((int)g.ball_x, (int)g.ball_y, BALL_R, WHITE);
            DrawText(TextFormat("%d", g.score0), SCREEN_W/4 - 20, 40, 40, GRAY);
            DrawText(TextFormat("%d", g.score1), 3*SCREEN_W/4 - 20, 40, 40, GRAY);
        } else if (g.screen == GameScreen::Error) {
            DrawText("Error", 80, 80, 32, RED);
            DrawText(g.error_msg.c_str(), 80, 140, 18, LIGHTGRAY);
            DrawText("Press SPACE to return to menu", 80, 220, 20, GRAY);
            if (IsKeyPressed(KEY_SPACE)) {
                g.screen = GameScreen::Menu;
                g.error_msg.clear();
                g.entering_join_code = false;
                g.join_code_input.clear();
            }
        }

        EndDrawing();
    }

    g.client.disconnect();
    CloseWindow();
    return 0;
}
