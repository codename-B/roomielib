/// @description Roomie: Reusable TCP client for Roomie protocol. Use network_connect_raw_async; handle Async Network in your object and call roomie_handle_async; call roomie_client_poll every step.

#macro ROOMIE_MAX_FRAME 65536

/// Create a new client instance. Set .callbacks before connect.
function roomie_client_create() {
	var c = {};
	c[$ "socket"] = -1;
	c[$ "connected"] = false;
	c[$ "is_ws"] = false; // true when connected via WebSocket (HTML5)
	c[$ "room_code"] = "";
	c[$ "peer_id"] = 0;
	c[$ "host_peer_id"] = 0;
	c[$ "side"] = 0;
	c[$ "last_error"] = "";
	c[$ "recv_buf"] = buffer_create(4096, buffer_grow, 1);
	c[$ "recv_read_pos"] = 0;
	c[$ "callbacks"] = {}; // on_joined, on_peer_joined, on_peer_left, on_peer_input, on_state_update, on_move_accepted, on_move_rejected, on_host_transferred, on_kicked, on_banned, on_settings_changed, on_pong, on_error, on_room_list
	return c;
}

/// Call from your object's Async Network event with async_load. Handles connect result and incoming data.
function roomie_handle_async(client, async_map) {
	// async_load is a DS map — must use [? "key"] not [$ "key"]
	var type = async_map[? "type"];
	show_debug_message("[roomie] handle_async type=" + string(type) + " socket=" + string(client[$ "socket"]));
	if (type == network_type_non_blocking_connect) {
		var succ = async_map[? "succeeded"];
		show_debug_message("[roomie] non_blocking_connect succeeded=" + string(succ));
		client[$ "connected"] = (succ == 1);
		if (!client[$ "connected"])
			client[$ "last_error"] = "Connection failed or timed out";
		return;
	}
	if (type == network_type_disconnect) {
		show_debug_message("[roomie] disconnect event");
		client[$ "connected"] = false;
		client[$ "socket"] = -1;
		client[$ "room_code"] = "";
		client[$ "peer_id"] = 0;
		client[$ "host_peer_id"] = 0;
		if (client[$ "callbacks"][$ "on_error"] != undefined)
			client[$ "callbacks"][$ "on_error"]("Connection lost");
		return;
	}
	if (type == network_type_data) {
		var sid = async_map[? "id"];
		var in_buff = async_map[? "buffer"];
		show_debug_message("[roomie] data event sid=" + string(sid) + " our_sock=" + string(client[$ "socket"]) + " buff_type=" + typeof(in_buff));
		if (sid != undefined && sid != client[$ "socket"]) { show_debug_message("[roomie] SKIP: socket mismatch"); return; }
		if (in_buff != undefined) {
			var size = buffer_get_size(in_buff);
			show_debug_message("[roomie] incoming data bytes=" + string(size));
			if (size > 0) {
				var recv = client[$ "recv_buf"];
				var at = buffer_get_size(recv);
				buffer_seek(recv, buffer_seek_start, at);
				buffer_copy(in_buff, 0, size, recv, at);
				show_debug_message("[roomie] recv_buf now=" + string(buffer_get_size(recv)) + " bytes");
			}
		}
		return;
	}
	show_debug_message("[roomie] unhandled async type=" + string(type));
}

/// Send one framed message (4-byte LE length + payload). Frees payload_buff after send.
function _roomie_send_frame(client, payload_buff) {
	if (client[$ "socket"] < 0 || !client[$ "connected"]) { show_debug_message("[roomie] send_frame: not connected"); return false; }
	var plen = buffer_tell(payload_buff);
	show_debug_message("[roomie] send_frame: payload_len=" + string(plen));
	if (plen > ROOMIE_MAX_FRAME) {
		buffer_delete(payload_buff);
		return false;
	}
	var frame = buffer_create(4 + plen, buffer_fixed, 1);
	buffer_seek(frame, buffer_seek_start, 0);
	buffer_write(frame, buffer_u32, plen);
	buffer_copy(payload_buff, 0, plen, frame, 4);
	var sent = network_send_raw(client[$ "socket"], frame, 4 + plen);
	show_debug_message("[roomie] send_frame: network_send_raw returned " + string(sent));
	var ok = (sent >= 0);
	buffer_delete(frame);
	buffer_delete(payload_buff);
	return ok;
}

/// Connect (async). Use roomie_handle_async in Async Network event to confirm; we set connected true here so UI can advance immediately.
/// Uses TCP on desktop; on HTML5 (browser) uses WebSocket (ws://). Pass tcp_port and ws_port so the correct port is used per transport.
function roomie_client_connect(client, host, tcp_port, ws_port) {
	roomie_client_disconnect(client);
	var use_ws = (os_browser != browser_not_a_browser);
	var port = tcp_port;
	if (use_ws)
		port = ws_port;
	var sock = -1;
	if (use_ws) {
		sock = network_create_socket(network_socket_ws);
		client[$ "is_ws"] = true;
	} else {
		sock = network_create_socket(network_socket_tcp);
		client[$ "is_ws"] = false;
	}
	if (sock < 0) {
		client[$ "last_error"] = "Failed to create socket";
		return false;
	}
	client[$ "socket"] = sock;
	client[$ "last_error"] = "";
	var err = -1;
	if (use_ws) {
		var url = "ws://" + host + ":" + string(real(port));
		err = network_connect_raw_async(sock, url, 0);
	} else {
		err = network_connect_raw_async(sock, host, real(port));
	}
	if (err < 0) {
		network_destroy(sock);
		client[$ "socket"] = -1;
		client[$ "last_error"] = "Connect failed";
		return false;
	}
	// Optimistic: show "Create/Join" immediately; Async Network event will set the real result when it fires
	client[$ "connected"] = true;
	return true;
}

function roomie_client_disconnect(client) {
	if (client[$ "socket"] >= 0) {
		network_destroy(client[$ "socket"]);
		client[$ "socket"] = -1;
	}
	client[$ "connected"] = false;
	client[$ "room_code"] = "";
	client[$ "peer_id"] = 0;
	client[$ "host_peer_id"] = 0;
	client[$ "recv_read_pos"] = 0;
	buffer_seek(client[$ "recv_buf"], buffer_seek_start, 0);
	buffer_resize(client[$ "recv_buf"], 0);
}

function roomie_client_is_connected(client) {
	return client[$ "connected"] && client[$ "socket"] >= 0;
}

function roomie_client_is_host(client) {
	return client[$ "peer_id"] != 0 && client[$ "peer_id"] == client[$ "host_peer_id"];
}

/// Create room (Hello with empty room_code)
function roomie_client_create_room(client, app_key, client_hash, display_name, auth_token, password) {
	if (!roomie_client_is_connected(client)) return false;
	var payload = roomie_encode_hello(app_key, "", client_hash, display_name, auth_token, password);
	return _roomie_send_frame(client, payload);
}

/// Join room by code
function roomie_client_join_room(client, app_key, room_code, client_hash, display_name, auth_token, password) {
	if (!roomie_client_is_connected(client)) return false;
	var payload = roomie_encode_hello(app_key, room_code, client_hash, display_name, auth_token, password);
	return _roomie_send_frame(client, payload);
}

/// Send state blob (host only). blob: string or buffer
function roomie_client_send_move(client, blob) {
	if (!roomie_client_is_connected(client)) return false;
	var payload = roomie_encode_move(blob);
	return _roomie_send_frame(client, payload);
}

/// Send input blob (forwarded to host as PeerInput)
function roomie_client_send_input(client, blob) {
	if (!roomie_client_is_connected(client)) return false;
	var payload = roomie_encode_input(blob);
	return _roomie_send_frame(client, payload);
}

function roomie_client_request_state(client) {
	if (!roomie_client_is_connected(client)) return false;
	var payload = roomie_encode_request_state();
	return _roomie_send_frame(client, payload);
}

function roomie_client_list_rooms(client, app_key) {
	if (!roomie_client_is_connected(client)) return false;
	var payload = roomie_encode_list_rooms(app_key);
	return _roomie_send_frame(client, payload);
}

function roomie_client_set_room_public(client, is_public) {
	if (!roomie_client_is_connected(client)) return false;
	var payload = roomie_encode_room_control_set_public(is_public);
	return _roomie_send_frame(client, payload);
}

function roomie_client_leave_room(client) {
	if (!roomie_client_is_connected(client)) return false;
	var payload = roomie_encode_room_control_leave();
	var ok = _roomie_send_frame(client, payload);
	client[$ "room_code"] = "";
	client[$ "peer_id"] = 0;
	client[$ "host_peer_id"] = 0;
	return ok;
}

/// Process received data and dispatch callbacks. Call every step.
function roomie_client_poll(client) {
	if (client[$ "socket"] < 0) return;
	var recv = client[$ "recv_buf"];
	var rpos = client[$ "recv_read_pos"];
	var total = buffer_get_size(recv);
	var size = total - rpos;
	if (size < 4) return;
	show_debug_message("[roomie] poll: recv_buf total=" + string(total) + " rpos=" + string(rpos) + " avail=" + string(size));
	var frame_len = buffer_peek(recv, rpos, buffer_u32);
	show_debug_message("[roomie] poll: frame_len=" + string(frame_len));
	if (frame_len > ROOMIE_MAX_FRAME) {
		show_debug_message("[roomie] poll: FRAME TOO LARGE");
		client[$ "recv_read_pos"] = total;
		if (client[$ "callbacks"][$ "on_error"] != undefined)
			client[$ "callbacks"][$ "on_error"]("Frame too large");
		return;
	}
	if (size < 4 + frame_len) { show_debug_message("[roomie] poll: incomplete frame, need " + string(4 + frame_len) + " have " + string(size)); return; }
	var payload = buffer_create(frame_len, buffer_fixed, 1);
	buffer_copy(recv, rpos + 4, frame_len, payload, 0);
	client[$ "recv_read_pos"] = rpos + 4 + frame_len;
	buffer_seek(payload, buffer_seek_start, 0);
	var msg = roomie_decode_server_message(payload);
	buffer_delete(payload);
	show_debug_message("[roomie] poll: decoded tag=" + string(msg[$ "tag"]));
	if (msg[$ "tag"] == 0) {
		client[$ "room_code"] = msg[$ "room_code"] ?? "";
		client[$ "peer_id"] = msg[$ "peer_id"] ?? 0;
		client[$ "host_peer_id"] = msg[$ "host_peer_id"] ?? 0;
		client[$ "side"] = msg[$ "side"] ?? 0;
		var init_state = msg[$ "initial_state"];
		var init_str; if (init_state != undefined) init_str = roomie_blob_to_string(init_state); else init_str = "";
		if (init_state != undefined) buffer_delete(init_state);
		if (client[$ "callbacks"][$ "on_joined"] != undefined)
			client[$ "callbacks"][$ "on_joined"](client[$ "room_code"], client[$ "peer_id"], client[$ "host_peer_id"], client[$ "side"], msg[$ "settings"] ?? {}, init_str);
	} else if (msg[$ "tag"] == 1 && client[$ "callbacks"][$ "on_room_list"] != undefined)
		client[$ "callbacks"][$ "on_room_list"](msg[$ "rooms"] ?? []);
	else if (msg[$ "tag"] == 2 && client[$ "callbacks"][$ "on_peer_joined"] != undefined)
		client[$ "callbacks"][$ "on_peer_joined"](msg[$ "peer_id"], msg[$ "client_hash"] ?? "", msg[$ "display_name"]);
	else if (msg[$ "tag"] == 3 && client[$ "callbacks"][$ "on_peer_left"] != undefined)
		client[$ "callbacks"][$ "on_peer_left"](msg[$ "peer_id"]);
	else if (msg[$ "tag"] == 4 && client[$ "callbacks"][$ "on_peer_input"] != undefined) {
		var blob = msg[$ "blob"];
		if (blob != undefined) {
			var blob_str = roomie_blob_to_string(blob);
			buffer_delete(blob);
			client[$ "callbacks"][$ "on_peer_input"](msg[$ "peer_id"], blob_str);
		}
	} else if (msg[$ "tag"] == 5 && client[$ "callbacks"][$ "on_state_update"] != undefined) {
		var blob = msg[$ "blob"];
		if (blob != undefined) {
			var blob_str = roomie_blob_to_string(blob);
			buffer_delete(blob);
			client[$ "callbacks"][$ "on_state_update"](blob_str);
		}
	} else if (msg[$ "tag"] == 6 && client[$ "callbacks"][$ "on_move_accepted"] != undefined)
		client[$ "callbacks"][$ "on_move_accepted"]();
	else if (msg[$ "tag"] == 7 && client[$ "callbacks"][$ "on_move_rejected"] != undefined)
		client[$ "callbacks"][$ "on_move_rejected"](msg[$ "reason"] ?? "");
	else if (msg[$ "tag"] == 8) {
		var et = msg[$ "event_tag"] ?? 0;
		if (et == 0 && client[$ "callbacks"][$ "on_host_transferred"] != undefined) {
			client[$ "host_peer_id"] = msg[$ "new_host_peer_id"] ?? 0;
			client[$ "callbacks"][$ "on_host_transferred"](client[$ "host_peer_id"]);
		} else if (et == 1 && client[$ "callbacks"][$ "on_kicked"] != undefined)
			client[$ "callbacks"][$ "on_kicked"]();
		else if (et == 2 && client[$ "callbacks"][$ "on_banned"] != undefined)
			client[$ "callbacks"][$ "on_banned"]();
		else if (et == 3 && client[$ "callbacks"][$ "on_settings_changed"] != undefined)
			client[$ "callbacks"][$ "on_settings_changed"](msg[$ "settings"] ?? {});
	} else if (msg[$ "tag"] == 9 && client[$ "callbacks"][$ "on_pong"] != undefined)
		client[$ "callbacks"][$ "on_pong"]();
	else if (msg[$ "tag"] == 10 && client[$ "callbacks"][$ "on_error"] != undefined)
		client[$ "callbacks"][$ "on_error"](msg[$ "message"] ?? "");
	if (client[$ "recv_read_pos"] >= 2048) {
		var remain = total - client[$ "recv_read_pos"];
		if (remain > 0) {
			var tmp = buffer_create(remain, buffer_fixed, 1);
			buffer_copy(recv, client[$ "recv_read_pos"], remain, tmp, 0);
			buffer_copy(tmp, 0, remain, recv, 0);
			buffer_delete(tmp);
		}
		buffer_resize(recv, remain);
		client[$ "recv_read_pos"] = 0;
	}
	roomie_client_poll(client);
}
