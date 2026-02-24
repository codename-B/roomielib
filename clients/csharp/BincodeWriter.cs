using System;
using System.Collections.Generic;
using System.Text;

namespace Roomie
{
    /// <summary>
    /// Encodes values using bincode's varint encoding format.
    /// Matches bincode DefaultOptions::new().with_varint_encoding().
    /// </summary>
    public class BincodeWriter
    {
        private readonly List<byte> _buf = new List<byte>();

        public void WriteVarint(ulong v)
        {
            if (v < 251)
            {
                _buf.Add((byte)v);
            }
            else if (v < (1UL << 16))
            {
                _buf.Add(251);
                _buf.Add((byte)v);
                _buf.Add((byte)(v >> 8));
            }
            else if (v < (1UL << 32))
            {
                _buf.Add(252);
                _buf.Add((byte)v);
                _buf.Add((byte)(v >> 8));
                _buf.Add((byte)(v >> 16));
                _buf.Add((byte)(v >> 24));
            }
            else
            {
                _buf.Add(253);
                for (int i = 0; i < 8; i++)
                {
                    _buf.Add((byte)(v >> (i * 8)));
                }
            }
        }

        public void WriteU8(byte v) => _buf.Add(v);

        public void WriteBool(bool v) => _buf.Add(v ? (byte)1 : (byte)0);

        public void WriteString(string s)
        {
            byte[] utf8 = Encoding.UTF8.GetBytes(s);
            WriteVarint((ulong)utf8.Length);
            _buf.AddRange(utf8);
        }

        public void WriteOptString(string s)
        {
            if (s == null)
            {
                WriteU8(0); // None
            }
            else
            {
                WriteU8(1); // Some
                WriteString(s);
            }
        }

        public void WriteBytes(byte[] data)
        {
            WriteVarint((ulong)data.Length);
            _buf.AddRange(data);
        }

        public byte[] ToArray() => _buf.ToArray();

        public void Clear() => _buf.Clear();

        public int Length => _buf.Count;
    }
}
