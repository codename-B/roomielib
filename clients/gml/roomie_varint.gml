/// @description Roomie: Bincode-style varint (matches Rust bincode with_varint_encoding). 0-250=1 byte; 251+2B LE; 252+4B LE; 253+8B LE.

/// Write unsigned integer in bincode varint form.
function roomie_varint_write(buff, value) {
	var val = floor(abs(value)) & $FFFFFFFF;
	if (val < 251) {
		buffer_write(buff, buffer_u8, val);
		return;
	}
	if (val < 65536) {
		buffer_write(buff, buffer_u8, 251);
		buffer_write(buff, buffer_u16, val);
		return;
	}
	buffer_write(buff, buffer_u8, 252);
	buffer_write(buff, buffer_u32, val);
}

/// Read one bincode varint; advances buffer position.
function roomie_varint_read(buff) {
	var b = buffer_read(buff, buffer_u8);
	if (b < 251) return b;
	if (b == 251) return buffer_read(buff, buffer_u16);
	if (b == 252) return buffer_read(buff, buffer_u32);
	if (b == 253) {
		var lo = buffer_read(buff, buffer_u32);
		var hi = buffer_read(buff, buffer_u32);
		return lo + (hi * 4294967296);
	}
	return 0;
}
