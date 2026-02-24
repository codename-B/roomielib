# Roomie

Add multiplayer to any game. Free server at `server.roomielib.com`.

One player is the **host** — they send game state. Everyone else sends input. The server relays both. See `clients/cpp/examples/raylib_pong/` for a complete example.

## C++ — `clients/cpp/`

```cpp
roomie::RoomieClient client;
roomie::ClientCallbacks cbs;

cbs.on_joined = [](auto& room_code, auto peer_id, auto host_peer_id, auto side, auto&, auto* state, auto len) {
    // you're in
};
cbs.on_state_update = [](const uint8_t* data, size_t len) {
    // new game state from host
};
cbs.on_peer_input = [](uint32_t peer_id, const uint8_t* data, size_t len) {
    // input from a peer (host only)
};

client.set_callbacks(std::move(cbs));
client.connect("server.roomielib.com", "8765");
client.create_room("pong", my_hash, nullptr);

// each frame
client.poll();
if (client.is_host())
    client.send_move(state_bytes, len);
else
    client.send_input(input_bytes, len);
```

Build: `cd clients/cpp && cmake -B build && cmake --build build`

Full example: `clients/cpp/examples/raylib_pong/main.cpp`

## C# — `clients/csharp/`

```csharp
var client = new Roomie.RoomieClient();

client.OnJoined += args => { /* args.RoomCode, args.PeerId, args.Side */ };
client.OnStateUpdate += data => { /* byte[] game state from host */ };
client.OnPeerInput += args => { /* args.PeerId, args.Blob (host only) */ };

client.Connect("server.roomielib.com", 8765, 8766);
client.CreateRoom("pong", myHash);

// each frame
client.Poll();
if (client.IsHost) client.SendMove(stateBytes);
else               client.SendInput(inputBytes);
```

Drop the `.cs` files into your Unity or .NET project. WebGL builds need `RoomieWebSocket.jslib` in `Plugins/WebGL/`.

## GML — `clients/gml/`

```gml
client = roomie_client_create();
client.callbacks.on_joined = function(code, pid, host, side, settings, state) { /* in */ };
client.callbacks.on_state_update = function(blob) { /* parse with roomie_blob_to_string */ };
client.callbacks.on_peer_input = function(pid, blob) { /* host only */ };

roomie_client_connect(client, "server.roomielib.com", 8765, 8766);
roomie_client_create_room(client, "pong", my_hash);

// Async Network event
roomie_handle_async(client, async_load);

// Step
roomie_client_poll(client);
if (client.peer_id == client.host_peer_id)
    roomie_client_send_move(client, json_stringify(state));
else
    roomie_client_send_input(client, json_stringify(input));
```

Copy the four `.gml` scripts into your project.

## License

MIT
