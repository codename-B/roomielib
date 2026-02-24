/// @description Roomie: Decode ServerMessage payload. Source: src/protocol/messages.rs
/// Payload buffer: seek to 0, then read varint tag and fields. Returns struct with .tag and tag-specific fields.

/// Read string (varint length + UTF-8 bytes). Advances buffer position. Returns string.
function _roomie_read_str(buff) {
	var len = roomie_varint_read(buff);
	if (len <= 0) return "";
	if (buffer_tell(buff) + len > buffer_get_size(buff)) return "";
	var temp = buffer_create(len + 1, buffer_fixed, 1);
	buffer_copy(buff, buffer_tell(buff), len, temp, 0);
	buffer_seek(buff, buffer_seek_start, buffer_tell(buff) + len);
	buffer_seek(temp, buffer_seek_start, len);
	buffer_write(temp, buffer_u8, 0);
	buffer_seek(temp, buffer_seek_start, 0);
	var s = buffer_read(temp, buffer_text);
	buffer_delete(temp);
	return s;
}

/// Option<String>: 0 = None, 1 = Some + string
function _roomie_read_opt_str(buff) {
	var tag = buffer_read(buff, buffer_u8);
	if (tag == 0) return undefined;
	return _roomie_read_str(buff);
}

/// Skip one bincode-encoded serde_json::Value (0=Null, 1=Bool, 2=Number, 3=String, 4=Array, 5=Object)
function _roomie_skip_bincode_json_value(buff) {
	var tag = roomie_varint_read(buff);
	if (tag == 0) return; // Null
	if (tag == 1) { buffer_read(buff, buffer_u8); return; } // Bool
	if (tag == 2) { buffer_seek(buff, buffer_seek_relative, 8); return; } // Number (f64)
	if (tag == 3) { var n = roomie_varint_read(buff); buffer_seek(buff, buffer_seek_relative, n); return; } // String
	if (tag == 4) {
		var n = roomie_varint_read(buff);
		repeat (n) { if (buffer_tell(buff) >= buffer_get_size(buff)) break; _roomie_skip_bincode_json_value(buff); }
		return;
	}
	if (tag == 5) {
		var n = roomie_varint_read(buff);
		repeat (n) {
			if (buffer_tell(buff) >= buffer_get_size(buff)) break;
			_roomie_read_str(buff); // key
			_roomie_skip_bincode_json_value(buff); // value
		}
		return;
	}
}

/// RoomSettings (skip custom if Some)
function _roomie_read_room_settings(buff) {
	var s = {};
	s[$ "max_players"] = roomie_varint_read(buff);
	s[$ "min_players"] = roomie_varint_read(buff);
	s[$ "is_public"] = buffer_read(buff, buffer_u8) != 0;
	s[$ "password"] = _roomie_read_opt_str(buff);
	s[$ "accept_required"] = buffer_read(buff, buffer_u8) != 0;
	var has_custom = buffer_read(buff, buffer_u8);
	if (has_custom != 0) _roomie_skip_bincode_json_value(buff);
	return s;
}

/// Vec<u8>: varint length + bytes. Returns buffer.
function _roomie_read_bytes(buff) {
	var len = roomie_varint_read(buff);
	if (len <= 0 || buffer_tell(buff) + len > buffer_get_size(buff)) return undefined;
	var out = buffer_create(len, buffer_fixed, 1);
	buffer_copy(buff, buffer_tell(buff), len, out, 0);
	buffer_seek(buff, buffer_seek_start, buffer_tell(buff) + len);
	return out;
}

/// Option<Vec<u8>>: 0 = None, 1 = Some + varint len + bytes. Returns buffer or undefined.
function _roomie_read_opt_bytes(buff) {
	var tag = buffer_read(buff, buffer_u8);
	if (tag == 0) return undefined;
	return _roomie_read_bytes(buff);
}

/// Decode one ServerMessage. Buffer position at start of payload. Returns struct: .tag (0..9), and tag-specific fields.
function roomie_decode_server_message(buff) {
	var out = {};
	out[$ "tag"] = roomie_varint_read(buff);
	var tag = out[$ "tag"];

	switch (tag) {
		case 0: // Joined
			out[$ "room_code"] = _roomie_read_str(buff);
			out[$ "peer_id"] = roomie_varint_read(buff);
			out[$ "host_peer_id"] = roomie_varint_read(buff);
			out[$ "side"] = buffer_read(buff, buffer_u8);
			out[$ "settings"] = _roomie_read_room_settings(buff);
			out[$ "initial_state"] = _roomie_read_opt_bytes(buff);
			break;
		case 1: { // RoomList
			var n = roomie_varint_read(buff);
			var list = [];
			for (var i = 0; i < n && buffer_tell(buff) < buffer_get_size(buff); i++) {
				var e = {};
				e[$ "room_code"] = _roomie_read_str(buff);
				e[$ "player_count"] = roomie_varint_read(buff);
				e[$ "settings"] = _roomie_read_room_settings(buff);
				array_push(list, e);
			}
			out[$ "rooms"] = list;
			break;
		}
		case 2: // PeerJoined
			out[$ "peer_id"] = roomie_varint_read(buff);
			out[$ "client_hash"] = _roomie_read_str(buff);
			out[$ "display_name"] = _roomie_read_opt_str(buff);
			break;
		case 3: // PeerLeft
			out[$ "peer_id"] = roomie_varint_read(buff);
			break;
		case 4: // PeerInput
			out[$ "peer_id"] = roomie_varint_read(buff);
			out[$ "blob"] = _roomie_read_bytes(buff);
			break;
		case 5: // StateUpdate (Vec<u8> only, no Option)
			out[$ "blob"] = _roomie_read_bytes(buff);
			break;
		case 6: // MoveAccepted
			break;
		case 7: // MoveRejected
			out[$ "reason"] = _roomie_read_str(buff);
			break;
		case 8: // RoomEvent
			out[$ "event_tag"] = roomie_varint_read(buff);
			if (out[$ "event_tag"] == 0) out[$ "new_host_peer_id"] = roomie_varint_read(buff);
			else if (out[$ "event_tag"] == 3) out[$ "settings"] = _roomie_read_room_settings(buff);
			break;
		case 9: // Pong
			break;
		case 10: // Error
			out[$ "message"] = _roomie_read_str(buff);
			break;
		default:
			break;
	}
	return out;
}

/// Helper: convert blob buffer to UTF-8 string (for JSON state/input). Caller can json_parse the result.
function roomie_blob_to_string(blob_buff) {
	if (blob_buff == undefined) return "";
	var len = buffer_get_size(blob_buff);
	if (len <= 0) return "";
	var temp = buffer_create(len + 1, buffer_fixed, 1);
	buffer_copy(blob_buff, 0, len, temp, 0);
	buffer_seek(temp, buffer_seek_start, len);
	buffer_write(temp, buffer_u8, 0);
	buffer_seek(temp, buffer_seek_start, 0);
	var s = buffer_read(temp, buffer_text);
	buffer_delete(temp);
	return s;
}
