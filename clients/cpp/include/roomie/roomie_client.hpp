/**
 * Roomie C++ client implementation.
 * Protocol: 4-byte LE frame length + bincode payload
 */
#ifndef ROOMIE_ROOMIE_CLIENT_HPP
#define ROOMIE_ROOMIE_CLIENT_HPP

#include <cstdint>
#include <functional>
#include <memory>
#include <string>
#include <vector>

namespace roomie {

struct RoomSettings {
    uint32_t max_players = 0;
    uint32_t min_players = 0;
    bool is_public = false;
    std::string password;
    bool accept_required = false;
};

/** One room in the list from list_rooms() / RoomList. Password is never set. */
struct RoomListEntry {
    std::string room_code;
    uint32_t player_count = 0;
    RoomSettings settings;
};

/** Callbacks invoked from poll() when server messages arrive. All optional. */
struct ClientCallbacks {
    std::function<void(const std::string& room_code, uint32_t peer_id, uint32_t host_peer_id,
                       uint8_t side, const RoomSettings& settings, const uint8_t* initial_state, size_t state_len)>
        on_joined;
    std::function<void(uint32_t peer_id, const std::string& client_hash, const std::string* display_name)>
        on_peer_joined;
    std::function<void(uint32_t peer_id)> on_peer_left;
    std::function<void(uint32_t peer_id, const uint8_t* data, size_t len)> on_peer_input;
    std::function<void(const uint8_t* data, size_t len)> on_state_update;
    std::function<void()> on_move_accepted;
    std::function<void(const std::string& reason)> on_move_rejected;
    std::function<void(uint32_t new_host_peer_id)> on_host_transferred;
    std::function<void()> on_kicked;
    std::function<void()> on_banned;
    std::function<void(const RoomSettings&)> on_settings_changed;
    std::function<void()> on_pong;
    std::function<void(const std::string& message)> on_error;
    /** Called when server responds to list_rooms() with RoomList. */
    std::function<void(const std::vector<RoomListEntry>&)> on_room_list;
};

class RoomieClient {
public:
    RoomieClient();
    ~RoomieClient();

    RoomieClient(const RoomieClient&) = delete;
    RoomieClient& operator=(const RoomieClient&) = delete;

    /** Set callbacks (can be called before or after connect). */
    void set_callbacks(ClientCallbacks cbs);

    /**
     * Connect to server. Blocks until connected or failure.
     * Returns true on success, false on failure (check last_error()).
     */
    bool connect(const std::string& host, const std::string& port);

    /** Disconnect and clear session. */
    void disconnect();

    bool is_connected() const;

    /**
     * Create a new room (empty room_code). Send Hello with app_key and "".
     * You become host. Call after connect().
     * auth_token: required when server sets ROOMIE_APP_SECRET.
     */
    bool create_room(const std::string& app_key,
                     const std::string& client_hash,
                     const std::string* display_name = nullptr,
                     const std::string* auth_token = nullptr,
                     const std::string* password = nullptr);

    /**
     * Join existing room by code. Call after connect().
     * password: required when the room has a password set.
     * auth_token: required when server sets ROOMIE_APP_SECRET.
     */
    bool join_room(const std::string& app_key,
                   const std::string& room_code,
                   const std::string& client_hash,
                   const std::string* display_name = nullptr,
                   const std::string* auth_token = nullptr,
                   const std::string* password = nullptr);

    /** Send a move (state blob). Only host's moves are accepted by server. */
    bool send_move(const uint8_t* data, size_t len);
    bool send_move(const std::vector<uint8_t>& data);

    /** Send input blob (e.g. paddle position). Forwarded to host only as PeerInput. */
    bool send_input(const uint8_t* data, size_t len);
    bool send_input(const std::vector<uint8_t>& data);

    /** Request full state (e.g. after becoming host). */
    bool request_state();

    /** Send ping (server replies with Pong). */
    bool send_ping();

    /**
     * Request list of open public rooms for an app. Call after connect().
     * Response is delivered via on_room_list callback (call poll() to receive it).
     */
    bool list_rooms(const std::string& app_key);

    /** Set current room public (true) or hidden (false). Host only. Makes room visible in list_rooms(). */
    bool set_room_public(bool is_public);

    /** Leave current room. */
    bool leave_room();

    /**
     * Process incoming data and dispatch callbacks. Call every game frame.
     * Non-blocking (uses whatever data is available).
     */
    void poll();

    /** Last error string (connection or encode/decode). */
    const std::string& last_error() const;

    /** Current room code (after Joined). Empty if not in a room. */
    const std::string& room_code() const;

    /** Our peer id (after Joined). 0 if not in a room. */
    uint32_t peer_id() const;

    /** Current host peer id (after Joined). */
    uint32_t host_peer_id() const;

    /** True if we are the host (peer_id() == host_peer_id()). */
    bool is_host() const;

private:
    struct Impl;
    std::unique_ptr<Impl> impl_;
};

} // namespace roomie

#endif
