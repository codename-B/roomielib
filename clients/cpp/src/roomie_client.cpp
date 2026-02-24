/**
 * Roomie C++ client implementation.
 * Protocol: 4-byte LE frame length + bincode payload
 */
#include "roomie/roomie_client.hpp"
#include <algorithm>
#include <cstring>
#include <vector>

#ifdef _WIN32
#define WIN32_LEAN_AND_MEAN
#include <winsock2.h>
#include <ws2tcpip.h>
#pragma comment(lib, "ws2_32.lib")
typedef SOCKET socket_t;
#define INVALID_SOCK INVALID_SOCKET
#define SOCK_ERR NO_ERROR
static int last_sock_err() { return WSAGetLastError(); }
static void close_sock(socket_t s) { if (s != INVALID_SOCK) closesocket(s); }
#else
#include <cerrno>
#include <fcntl.h>
#include <netdb.h>
#include <sys/socket.h>
#include <unistd.h>
typedef int socket_t;
#define INVALID_SOCK (-1)
#define SOCK_ERR (0)
static int last_sock_err() { return errno; }
static void close_sock(socket_t s) { if (s >= 0) close(s); }
#endif

namespace roomie {

static const size_t MAX_FRAME_LEN = 64 * 1024;

// --- Bincode varint encoding (matches bincode with_varint_encoding) ---
// u < 251: 1 byte; 251..65535: 0xFB + u16 LE; 2^16..2^32-1: 0xFC + u32 LE; 2^32..2^64-1: 0xFD + u64 LE
static void write_varint(std::vector<uint8_t>& out, uint64_t v) {
    if (v < 251) {
        out.push_back((uint8_t)v);
        return;
    }
    if (v < (1ull << 16)) {
        out.push_back(251);
        out.push_back((uint8_t)(v));
        out.push_back((uint8_t)(v >> 8));
        return;
    }
    if (v < (1ull << 32)) {
        out.push_back(252);
        out.push_back((uint8_t)(v));
        out.push_back((uint8_t)(v >> 8));
        out.push_back((uint8_t)(v >> 16));
        out.push_back((uint8_t)(v >> 24));
        return;
    }
    out.push_back(253);
    for (int i = 0; i < 8; i++) {
        out.push_back((uint8_t)(v));
        v >>= 8;
    }
}
static void write_u8(std::vector<uint8_t>& out, uint8_t v) {
    out.push_back(v);
}
static void write_str(std::vector<uint8_t>& out, const std::string& s) {
    write_varint(out, (uint64_t)s.size());
    out.insert(out.end(), s.begin(), s.end());
}
static void write_opt_str(std::vector<uint8_t>& out, const std::string* s) {
    if (!s || s->empty()) {
        write_u8(out, 0);
        return;
    }
    write_u8(out, 1);
    write_str(out, *s);
}
static void write_bytes(std::vector<uint8_t>& out, const uint8_t* data, size_t len) {
    write_varint(out, (uint64_t)len);
    out.insert(out.end(), data, data + len);
}

static void write_room_settings(std::vector<uint8_t>& out, const RoomSettings& s) {
    write_varint(out, s.max_players);
    write_varint(out, s.min_players);
    write_u8(out, s.is_public ? 1 : 0);
    write_opt_str(out, s.password.empty() ? nullptr : &s.password);
    write_u8(out, s.accept_required ? 1 : 0);
    write_u8(out, 0); // custom = None
}

// --- Reader (for decode) ---
struct Reader {
    const uint8_t* data;
    size_t size;
    size_t pos;

    bool eof() const { return pos >= size; }
    bool need(size_t n) const { return pos + n <= size; }

    uint8_t read_u8() {
        if (!need(1)) return 0;
        return data[pos++];
    }
    uint64_t read_varint() {
        if (!need(1)) return 0;
        uint8_t b = data[pos++];
        if (b < 251) return b;
        if (b == 251 && need(2)) {
            uint64_t v = (uint64_t)data[pos] | ((uint64_t)data[pos+1] << 8);
            pos += 2;
            return v;
        }
        if (b == 252 && need(4)) {
            uint64_t v = (uint64_t)data[pos] | ((uint64_t)data[pos+1]<<8) | ((uint64_t)data[pos+2]<<16) | ((uint64_t)data[pos+3]<<24);
            pos += 4;
            return v;
        }
        if (b == 253 && need(8)) {
            uint64_t v = 0;
            for (int i = 0; i < 8; i++) v |= (uint64_t)data[pos+i] << (i*8);
            pos += 8;
            return v;
        }
        return 0;
    }
    uint32_t read_u32_varint() {
        return (uint32_t)read_varint();
    }
    std::string read_str() {
        uint64_t len = read_varint();
        if (len > 0x7FFFFFFF || !need((size_t)len)) return "";
        std::string s(data + pos, data + pos + (size_t)len);
        pos += (size_t)len;
        return s;
    }
    bool read_opt_str(std::string* out) {
        if (read_u8() == 0) { if (out) out->clear(); return false; }
        if (out) *out = read_str();
        else (void)read_str();
        return true;
    }
    void read_bytes(std::vector<uint8_t>& out) {
        uint64_t len = read_varint();
        if (len > MAX_FRAME_LEN || !need((size_t)len)) { out.clear(); return; }
        out.assign(data + pos, data + pos + (size_t)len);
        pos += (size_t)len;
    }
};

// Skip one bincode-encoded serde_json::Value (for RoomSettings.custom)
static void skip_bincode_json_value(Reader& r);

static void read_room_settings(Reader& r, RoomSettings& s) {
    s.max_players = (uint32_t)r.read_varint();
    s.min_players = (uint32_t)r.read_varint();
    s.is_public = r.read_u8() != 0;
    r.read_opt_str(&s.password);
    s.accept_required = r.read_u8() != 0;
    if (r.read_u8() != 0) skip_bincode_json_value(r); // custom = Some(Value)
}

// Serde_json Value enum: 0=Null, 1=Bool, 2=Number, 3=String, 4=Array, 5=Object (bincode variant index)
static void skip_bincode_json_value(Reader& r) {
    uint64_t tag = r.read_varint();
    switch (tag) {
    case 0: break; // Null
    case 1: (void)r.read_u8(); break; // Bool
    case 2: if (r.need(8)) r.pos += 8; break; // Number (f64)
    case 3: { uint64_t n = r.read_varint(); if (r.need((size_t)n)) r.pos += (size_t)n; break; } // String
    case 4: { uint64_t n = r.read_varint(); for (uint64_t i = 0; i < n && !r.eof(); i++) skip_bincode_json_value(r); break; } // Array
    case 5: { uint64_t n = r.read_varint(); for (uint64_t i = 0; i < n && !r.eof(); i++) { (void)r.read_str(); skip_bincode_json_value(r); } break; } // Object
    default: break;
    }
}

// --- Encode ClientMessage ---
static std::vector<uint8_t> encode_hello(const std::string& app_key,
                                         const std::string& room_code,
                                         const std::string& client_hash,
                                         const std::string* display_name,
                                         const std::string* auth_token,
                                         const std::string* password) {
    std::vector<uint8_t> out;
    write_varint(out, 0); // ClientMessage variant 0 = Hello
    write_str(out, app_key);
    write_str(out, room_code);
    write_str(out, client_hash);
    write_opt_str(out, display_name);
    write_opt_str(out, auth_token);
    write_opt_str(out, password);
    return out;
}

static std::vector<uint8_t> encode_move(const uint8_t* data, size_t len) {
    std::vector<uint8_t> out;
    write_varint(out, 1); // Move
    write_bytes(out, data, len);
    return out;
}

static std::vector<uint8_t> encode_input(const uint8_t* data, size_t len) {
    std::vector<uint8_t> out;
    write_varint(out, 2); // Input
    write_bytes(out, data, len);
    return out;
}

static std::vector<uint8_t> encode_room_control_leave() {
    std::vector<uint8_t> out;
    write_varint(out, 3); // RoomControl
    write_varint(out, 8); // Leave
    return out;
}

static std::vector<uint8_t> encode_room_control_set_public(bool is_public) {
    std::vector<uint8_t> out;
    write_varint(out, 3); // RoomControl
    write_varint(out, 2); // SetPublic
    write_u8(out, is_public ? 1 : 0);
    return out;
}

static std::vector<uint8_t> encode_request_state() {
    std::vector<uint8_t> out;
    write_varint(out, 4); // RequestState
    return out;
}

static std::vector<uint8_t> encode_ping() {
    std::vector<uint8_t> out;
    write_varint(out, 5); // Ping
    return out;
}

static std::vector<uint8_t> encode_list_rooms(const std::string& app_key) {
    std::vector<uint8_t> out;
    write_varint(out, 6); // ListRooms
    write_str(out, app_key);
    return out;
}

// Context for updating client session state when decoding server messages
struct SessionUpdate {
    std::string* room_code;
    uint32_t* peer_id;
    uint32_t* host_peer_id;
};

// --- Decode ServerMessage and dispatch ---
static void decode_and_dispatch(const uint8_t* payload, size_t len, ClientCallbacks& cbs, SessionUpdate* su) {
    Reader r{ payload, len, 0 };
    uint64_t tag = r.read_varint();
    switch (tag) {
    case 0: { // Joined
        std::string room_code = r.read_str();
        uint32_t peer_id = r.read_u32_varint();
        uint32_t host_peer_id = r.read_u32_varint();
        uint8_t side = r.read_u8();
        if (su) {
            *su->room_code = room_code;
            *su->peer_id = peer_id;
            *su->host_peer_id = host_peer_id;
        }
        RoomSettings settings;
        read_room_settings(r, settings);
        std::vector<uint8_t> initial_state;
        if (r.read_u8() != 0) r.read_bytes(initial_state);
        if (cbs.on_joined) {
            cbs.on_joined(room_code, peer_id, host_peer_id, side, settings,
                          initial_state.empty() ? nullptr : initial_state.data(),
                          initial_state.size());
        }
        break;
    }
    case 1: { // RoomList
        uint64_t n = r.read_varint();
        std::vector<RoomListEntry> list;
        list.reserve((size_t)(n > 256 ? 256 : n));
        for (uint64_t i = 0; i < n && !r.eof(); i++) {
            RoomListEntry e;
            e.room_code = r.read_str();
            e.player_count = r.read_u32_varint();
            read_room_settings(r, e.settings);
            list.push_back(std::move(e));
        }
        if (cbs.on_room_list) cbs.on_room_list(list);
        break;
    }
    case 2: { // PeerJoined
        uint32_t peer_id = r.read_u32_varint();
        std::string client_hash = r.read_str();
        std::string display_name;
        bool has_display = r.read_opt_str(&display_name);
        if (cbs.on_peer_joined)
            cbs.on_peer_joined(peer_id, client_hash, has_display ? &display_name : nullptr);
        break;
    }
    case 3: { // PeerLeft
        uint32_t peer_id = r.read_u32_varint();
        if (cbs.on_peer_left) cbs.on_peer_left(peer_id);
        break;
    }
    case 4: { // PeerInput
        uint32_t peer_id = r.read_u32_varint();
        std::vector<uint8_t> data;
        r.read_bytes(data);
        if (cbs.on_peer_input && !data.empty())
            cbs.on_peer_input(peer_id, data.data(), data.size());
        break;
    }
    case 5: { // StateUpdate
        std::vector<uint8_t> data;
        r.read_bytes(data);
        if (cbs.on_state_update && !data.empty())
            cbs.on_state_update(data.data(), data.size());
        break;
    }
    case 6: // MoveAccepted
        if (cbs.on_move_accepted) cbs.on_move_accepted();
        break;
    case 7: { // MoveRejected
        std::string reason = r.read_str();
        if (cbs.on_move_rejected) cbs.on_move_rejected(reason);
        break;
    }
    case 8: { // RoomEvent
        uint64_t ev = r.read_varint();
        switch (ev) {
        case 0: { uint32_t id = r.read_u32_varint(); if (su) *su->host_peer_id = id; if (cbs.on_host_transferred) cbs.on_host_transferred(id); break; }
        case 1: if (cbs.on_kicked) cbs.on_kicked(); break;
        case 2: if (cbs.on_banned) cbs.on_banned(); break;
        case 3: { RoomSettings s; read_room_settings(r, s); if (cbs.on_settings_changed) cbs.on_settings_changed(s); break; }
        default: break;
        }
        break;
    }
    case 9: // Pong
        if (cbs.on_pong) cbs.on_pong();
        break;
    case 10: { // Error
        std::string msg = r.read_str();
        if (cbs.on_error) cbs.on_error(msg);
        break;
    }
    default:
        break;
    }
}

// --- Frame: 4-byte LE length + payload ---
static bool send_frame(socket_t sock, const uint8_t* payload, size_t len) {
    if (len > MAX_FRAME_LEN) return false;
    uint8_t len_buf[4];
    len_buf[0] = (uint8_t)(len);
    len_buf[1] = (uint8_t)(len >> 8);
    len_buf[2] = (uint8_t)(len >> 16);
    len_buf[3] = (uint8_t)(len >> 24);
#ifdef _WIN32
    if (send(sock, (const char*)len_buf, 4, 0) != 4) return false;
    size_t sent = 0;
    while (sent < len) {
        int n = send(sock, (const char*)(payload + sent), (int)(len - sent), 0);
        if (n <= 0) return false;
        sent += (size_t)n;
    }
#else
    if (::send(sock, len_buf, 4, 0) != 4) return false;
    size_t sent = 0;
    while (sent < len) {
        ssize_t n = ::send(sock, payload + sent, len - sent, 0);
        if (n <= 0) return false;
        sent += (size_t)n;
    }
#endif
    return true;
}

struct RoomieClient::Impl {
    socket_t sock = INVALID_SOCK;
    std::string last_error_;
    ClientCallbacks callbacks;
    std::string room_code_;
    uint32_t peer_id_ = 0;
    uint32_t host_peer_id_ = 0;
    std::vector<uint8_t> recv_buf;
    bool connected = false;

    void set_error(const std::string& msg) {
        last_error_ = msg;
    }

    void set_error_sock() {
#ifdef _WIN32
        char buf[256];
        last_error_ = "socket error: " + std::to_string(WSAGetLastError());
#else
        last_error_ = std::string("socket error: ") + strerror(errno);
#endif
    }

    bool ensure_connected() {
        if (sock == INVALID_SOCK || !connected) {
            set_error("not connected");
            return false;
        }
        return true;
    }

    void handle_disconnect() {
        if (sock != INVALID_SOCK) {
            close_sock(sock);
            sock = INVALID_SOCK;
        }
        connected = false;
        room_code_.clear();
        peer_id_ = 0;
        host_peer_id_ = 0;
        recv_buf.clear();
        set_error("Connection lost");
        if (callbacks.on_error) callbacks.on_error("Connection lost");
    }

    bool send_message(const std::vector<uint8_t>& payload) {
        if (!send_frame(sock, payload.data(), payload.size())) {
            handle_disconnect();
            return false;
        }
        return true;
    }

    bool send_hello(const std::string& app_key,
                    const std::string& room_code,
                    const std::string& client_hash,
                    const std::string* display_name,
                    const std::string* auth_token,
                    const std::string* password) {
        if (!ensure_connected()) return false;
        auto payload = encode_hello(app_key, room_code, client_hash, display_name, auth_token, password);
        if (!send_message(payload)) {
            set_error_sock();
            return false;
        }
        return true;
    }

    bool try_recv_frame() {
        uint8_t buf[4096];
#ifdef _WIN32
        int n = recv(sock, (char*)buf, sizeof(buf), 0);
        if (n > 0) {
            recv_buf.insert(recv_buf.end(), buf, buf + n);
        } else if (n == 0) {
            handle_disconnect();
            return false;
        } else {
            int err = WSAGetLastError();
            if (err != WSAEWOULDBLOCK) { handle_disconnect(); return false; }
        }
#else
        ssize_t n = ::recv(sock, buf, sizeof(buf), 0);
        if (n > 0) {
            recv_buf.insert(recv_buf.end(), buf, buf + n);
        } else if (n == 0) {
            handle_disconnect();
            return false;
        } else {
            if (errno != EAGAIN && errno != EWOULDBLOCK) { handle_disconnect(); return false; }
        }
#endif
        if (recv_buf.size() < 4) return false;
        uint32_t len = (uint32_t)recv_buf[0] | ((uint32_t)recv_buf[1]<<8) | ((uint32_t)recv_buf[2]<<16) | ((uint32_t)recv_buf[3]<<24);
        if (len > MAX_FRAME_LEN) return false;
        if (recv_buf.size() < 4 + len) return false;
        SessionUpdate su{ &room_code_, &peer_id_, &host_peer_id_ };
        decode_and_dispatch(recv_buf.data() + 4, len, callbacks, &su);
        recv_buf.erase(recv_buf.begin(), recv_buf.begin() + 4 + (size_t)len);
        return true;
    }
};

RoomieClient::RoomieClient() : impl_(std::make_unique<Impl>()) {
#ifdef _WIN32
    static bool wsa_init = false;
    if (!wsa_init) {
        WSADATA wsa;
        if (WSAStartup(MAKEWORD(2, 2), &wsa) == 0) wsa_init = true;
    }
#endif
}

RoomieClient::~RoomieClient() {
    disconnect();
}

void RoomieClient::set_callbacks(ClientCallbacks cbs) {
    impl_->callbacks = std::move(cbs);
}

bool RoomieClient::connect(const std::string& host, const std::string& port) {
    disconnect();
    struct addrinfo hints = {}, *res = nullptr;
    hints.ai_family = AF_UNSPEC;
    hints.ai_socktype = SOCK_STREAM;
#ifdef _WIN32
    if (getaddrinfo(host.c_str(), port.c_str(), &hints, &res) != 0) {
        impl_->set_error("getaddrinfo failed");
        return false;
    }
#else
    if (::getaddrinfo(host.c_str(), port.c_str(), &hints, &res) != 0) {
        impl_->set_error("getaddrinfo failed");
        return false;
    }
#endif
    socket_t s = INVALID_SOCK;
    for (struct addrinfo* p = res; p; p = p->ai_next) {
#ifdef _WIN32
        s = socket(p->ai_family, p->ai_socktype, p->ai_protocol);
#else
        s = ::socket(p->ai_family, p->ai_socktype, p->ai_protocol);
#endif
        if (s == INVALID_SOCK) continue;
#ifdef _WIN32
        if (::connect(s, p->ai_addr, (int)p->ai_addrlen) == 0) break;
#else
        if (::connect(s, p->ai_addr, p->ai_addrlen) == 0) break;
#endif
        close_sock(s);
        s = INVALID_SOCK;
    }
    freeaddrinfo(res);
    if (s == INVALID_SOCK) {
        impl_->set_error_sock();
        return false;
    }
#ifdef _WIN32
    u_long nonblock = 1;
    ioctlsocket(s, FIONBIO, &nonblock);
#else
    int flags = fcntl(s, F_GETFL, 0);
    fcntl(s, F_SETFL, flags | O_NONBLOCK);
#endif
    impl_->sock = s;
    impl_->connected = true;
    impl_->recv_buf.clear();
    impl_->room_code_.clear();
    impl_->peer_id_ = 0;
    impl_->host_peer_id_ = 0;
    return true;
}

void RoomieClient::disconnect() {
    if (impl_->sock != INVALID_SOCK) {
        close_sock(impl_->sock);
        impl_->sock = INVALID_SOCK;
    }
    impl_->connected = false;
    impl_->room_code_.clear();
    impl_->peer_id_ = 0;
    impl_->host_peer_id_ = 0;
}

bool RoomieClient::is_connected() const {
    return impl_->connected && impl_->sock != INVALID_SOCK;
}

bool RoomieClient::create_room(const std::string& app_key,
                               const std::string& client_hash,
                               const std::string* display_name,
                               const std::string* auth_token,
                               const std::string* password) {
    return impl_->send_hello(app_key, "", client_hash, display_name, auth_token, password);
}

bool RoomieClient::join_room(const std::string& app_key,
                              const std::string& room_code,
                              const std::string& client_hash,
                              const std::string* display_name,
                              const std::string* auth_token,
                              const std::string* password) {
    return impl_->send_hello(app_key, room_code, client_hash, display_name, auth_token, password);
}

bool RoomieClient::send_move(const uint8_t* data, size_t len) {
    if (!impl_->ensure_connected()) return false;
    auto payload = encode_move(data, len);
    if (!impl_->send_message(payload)) { impl_->set_error_sock(); return false; }
    return true;
}

bool RoomieClient::send_move(const std::vector<uint8_t>& data) {
    return send_move(data.data(), data.size());
}

bool RoomieClient::send_input(const uint8_t* data, size_t len) {
    if (!impl_->ensure_connected()) return false;
    auto payload = encode_input(data, len);
    if (!impl_->send_message(payload)) { impl_->set_error_sock(); return false; }
    return true;
}

bool RoomieClient::send_input(const std::vector<uint8_t>& data) {
    return send_input(data.data(), data.size());
}

bool RoomieClient::request_state() {
    if (!impl_->ensure_connected()) return false;
    auto payload = encode_request_state();
    if (!impl_->send_message(payload)) { impl_->set_error_sock(); return false; }
    return true;
}

bool RoomieClient::send_ping() {
    if (!impl_->ensure_connected()) return false;
    auto payload = encode_ping();
    if (!impl_->send_message(payload)) { impl_->set_error_sock(); return false; }
    return true;
}

bool RoomieClient::list_rooms(const std::string& app_key) {
    if (!impl_->ensure_connected()) return false;
    auto payload = encode_list_rooms(app_key);
    if (!impl_->send_message(payload)) { impl_->set_error_sock(); return false; }
    return true;
}

bool RoomieClient::set_room_public(bool is_public) {
    if (!impl_->ensure_connected()) return false;
    auto payload = encode_room_control_set_public(is_public);
    if (!impl_->send_message(payload)) { impl_->set_error_sock(); return false; }
    return true;
}

bool RoomieClient::leave_room() {
    if (!impl_->ensure_connected()) return false;
    auto payload = encode_room_control_leave();
    if (!impl_->send_message(payload)) { impl_->set_error_sock(); return false; }
    impl_->room_code_.clear();
    impl_->peer_id_ = 0;
    impl_->host_peer_id_ = 0;
    return true;
}

void RoomieClient::poll() {
    if (impl_->sock == INVALID_SOCK) return;
    while (impl_->try_recv_frame()) {}
}

const std::string& RoomieClient::last_error() const {
    return impl_->last_error_;
}

const std::string& RoomieClient::room_code() const {
    return impl_->room_code_;
}

uint32_t RoomieClient::peer_id() const {
    return impl_->peer_id_;
}

uint32_t RoomieClient::host_peer_id() const {
    return impl_->host_peer_id_;
}

bool RoomieClient::is_host() const {
    return impl_->peer_id_ != 0 && impl_->peer_id_ == impl_->host_peer_id_;
}

} // namespace roomie