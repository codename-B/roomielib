/// @description Roomie: Encode ClientMessage and frame for wire. Source: src/protocol/messages.rs

/// Write string (varint length + UTF-8 bytes, no null)
function _roomie_write_str(buff, s) {
	if (!is_string(s)) s = "";
	roomie_varint_write(buff, string_byte_length(s));
	if (string_byte_length(s) > 0)
		buffer_write(buff, buffer_text, s);
}

/// Option<String>: 0 = None, 1 = Some + string
function _roomie_write_opt_str(buff, s) {
	if (s == undefined || s == "" || !is_string(s)) {
		buffer_write(buff, buffer_u8, 0);
		return;
	}
	buffer_write(buff, buffer_u8, 1);
	_roomie_write_str(buff, s);
}

/// RoomSettings: max_players, min_players, is_public, password, accept_required, custom (None)
function _roomie_write_room_settings(buff, max_players, min_players, is_public, password, accept_required) {
	roomie_varint_write(buff, max_players ?? 0);
	roomie_varint_write(buff, min_players ?? 0);
	var _pub; if (is_public) _pub = 1; else _pub = 0;
	buffer_write(buff, buffer_u8, _pub);
	_roomie_write_opt_str(buff, password);
	var _acc; if (accept_required) _acc = 1; else _acc = 0;
	buffer_write(buff, buffer_u8, _acc);
	buffer_write(buff, buffer_u8, 0); // custom = None
}

/// ClientMessage variant 0 = Hello
function roomie_encode_hello(app_key, room_code, client_hash, display_name, auth_token, password) {
	var buff = buffer_create(256, buffer_grow, 1);
	roomie_varint_write(buff, 0); // Hello
	_roomie_write_str(buff, app_key);
	_roomie_write_str(buff, room_code ?? "");
	_roomie_write_str(buff, client_hash);
	_roomie_write_opt_str(buff, display_name);
	_roomie_write_opt_str(buff, auth_token);
	_roomie_write_opt_str(buff, password);
	return buff;
}

/// ClientMessage variant 1 = Move (state blob)
/// blob: string (UTF-8) or buffer; if buffer, it is read from start and size = buffer_get_size
function roomie_encode_move(blob) {
	var buff = buffer_create(64, buffer_grow, 1);
	roomie_varint_write(buff, 1); // Move
	var len = 0;
	if (is_string(blob)) {
		len = string_byte_length(blob);
		roomie_varint_write(buff, len);
		if (len > 0) buffer_write(buff, buffer_text, blob);
	} else if (typeof(blob) == "buffer") {
		len = buffer_get_size(blob);
		roomie_varint_write(buff, len);
		buffer_copy(blob, 0, len, buff, buffer_tell(buff));
	} else {
		roomie_varint_write(buff, 0);
	}
	return buff;
}

/// ClientMessage variant 2 = Input
function roomie_encode_input(blob) {
	var buff = buffer_create(64, buffer_grow, 1);
	roomie_varint_write(buff, 2); // Input
	var len = 0;
	if (is_string(blob)) {
		len = string_byte_length(blob);
		roomie_varint_write(buff, len);
		if (len > 0) buffer_write(buff, buffer_text, blob);
	} else if (typeof(blob) == "buffer") {
		len = buffer_get_size(blob);
		roomie_varint_write(buff, len);
		buffer_copy(blob, 0, len, buff, buffer_tell(buff));
	} else {
		roomie_varint_write(buff, 0);
	}
	return buff;
}

/// ClientMessage variant 3 = RoomControl, variant 8 = Leave
function roomie_encode_room_control_leave() {
	var buff = buffer_create(16, buffer_grow, 1);
	roomie_varint_write(buff, 3); // RoomControl
	roomie_varint_write(buff, 8); // Leave
	return buff;
}

/// ClientMessage variant 4 = RequestState
function roomie_encode_request_state() {
	var buff = buffer_create(16, buffer_grow, 1);
	roomie_varint_write(buff, 4);
	return buff;
}

/// ClientMessage variant 5 = Ping
function roomie_encode_ping() {
	var buff = buffer_create(16, buffer_grow, 1);
	roomie_varint_write(buff, 5);
	return buff;
}

/// ClientMessage variant 6 = ListRooms
function roomie_encode_list_rooms(app_key) {
	var buff = buffer_create(64, buffer_grow, 1);
	roomie_varint_write(buff, 6);
	_roomie_write_str(buff, app_key ?? "");
	return buff;
}

/// ClientMessage variant 3 = RoomControl, variant 2 = SetPublic
function roomie_encode_room_control_set_public(is_public) {
	var buff = buffer_create(16, buffer_grow, 1);
	roomie_varint_write(buff, 3);
	roomie_varint_write(buff, 2);
	buffer_write(buff, buffer_u8, is_public ? 1 : 0);
	return buff;
}
