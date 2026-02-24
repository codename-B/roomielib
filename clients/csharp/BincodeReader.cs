using System;
using System.Text;

namespace Roomie
{
    /// <summary>
    /// Decodes values using bincode's varint encoding format.
    /// Matches bincode DefaultOptions::new().with_varint_encoding().
    /// </summary>
    public class BincodeReader
    {
        private readonly byte[] _data;
        private readonly int _size;
        private int _pos;

        public BincodeReader(byte[] data, int offset, int length)
        {
            _data = data;
            _pos = offset;
            _size = offset + length;
        }

        public BincodeReader(byte[] data) : this(data, 0, data.Length) { }

        public bool Eof => _pos >= _size;
        public int Position => _pos;
        public int Remaining => _size - _pos;

        private bool Need(int n) => _pos + n <= _size;

        public byte ReadU8()
        {
            if (!Need(1)) throw new ProtocolException("Unexpected end of data reading u8");
            return _data[_pos++];
        }

        public bool ReadBool() => ReadU8() != 0;

        public ulong ReadVarint()
        {
            if (!Need(1)) throw new ProtocolException("Unexpected end of data reading varint");
            byte b = _data[_pos++];
            if (b < 251) return b;
            if (b == 251)
            {
                if (!Need(2)) throw new ProtocolException("Unexpected end of data reading u16 varint");
                ulong v = (ulong)_data[_pos] | ((ulong)_data[_pos + 1] << 8);
                _pos += 2;
                return v;
            }
            if (b == 252)
            {
                if (!Need(4)) throw new ProtocolException("Unexpected end of data reading u32 varint");
                ulong v = (ulong)_data[_pos]
                    | ((ulong)_data[_pos + 1] << 8)
                    | ((ulong)_data[_pos + 2] << 16)
                    | ((ulong)_data[_pos + 3] << 24);
                _pos += 4;
                return v;
            }
            if (b == 253)
            {
                if (!Need(8)) throw new ProtocolException("Unexpected end of data reading u64 varint");
                ulong v = 0;
                for (int i = 0; i < 8; i++)
                    v |= (ulong)_data[_pos + i] << (i * 8);
                _pos += 8;
                return v;
            }
            throw new ProtocolException($"Invalid varint prefix byte: {b}");
        }

        public uint ReadU32Varint() => (uint)ReadVarint();

        public string ReadString()
        {
            ulong len = ReadVarint();
            if (len > 0x7FFFFFFF)
                throw new ProtocolException($"String length too large: {len}");
            int n = (int)len;
            if (!Need(n)) throw new ProtocolException("Unexpected end of data reading string");
            string s = Encoding.UTF8.GetString(_data, _pos, n);
            _pos += n;
            return s;
        }

        /// <summary>
        /// Read Option&lt;String&gt;. Returns null for None.
        /// </summary>
        public string ReadOptString()
        {
            byte tag = ReadU8();
            if (tag == 0) return null;
            return ReadString();
        }

        public byte[] ReadBytes()
        {
            ulong len = ReadVarint();
            if (len > 65536)
                throw new ProtocolException($"Byte array length too large: {len}");
            int n = (int)len;
            if (!Need(n)) throw new ProtocolException("Unexpected end of data reading bytes");
            byte[] result = new byte[n];
            Buffer.BlockCopy(_data, _pos, result, 0, n);
            _pos += n;
            return result;
        }

        /// <summary>
        /// Recursively skip a bincode-encoded serde_json::Value.
        /// Used for RoomSettings.custom field.
        /// Variant tags: 0=Null, 1=Bool, 2=Number(f64), 3=String, 4=Array, 5=Object
        /// </summary>
        public void SkipBincodeJsonValue()
        {
            ulong tag = ReadVarint();
            switch (tag)
            {
                case 0: // Null
                    break;
                case 1: // Bool
                    ReadU8();
                    break;
                case 2: // Number (f64 = 8 bytes)
                    if (!Need(8)) throw new ProtocolException("Unexpected end of data reading JSON Number");
                    _pos += 8;
                    break;
                case 3: // String
                    ReadString();
                    break;
                case 4: // Array
                {
                    ulong n = ReadVarint();
                    for (ulong i = 0; i < n && !Eof; i++)
                        SkipBincodeJsonValue();
                    break;
                }
                case 5: // Object
                {
                    ulong n = ReadVarint();
                    for (ulong i = 0; i < n && !Eof; i++)
                    {
                        ReadString(); // key
                        SkipBincodeJsonValue(); // value
                    }
                    break;
                }
                default:
                    throw new ProtocolException($"Unknown JSON Value variant: {tag}");
            }
        }
    }

    public class ProtocolException : Exception
    {
        public ProtocolException(string message) : base(message) { }
    }
}
