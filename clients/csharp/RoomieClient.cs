using System;
using System.Collections.Generic;
#if UNITY_WEBGL && !UNITY_EDITOR
using System.Runtime.InteropServices;
#else
using System.Net.Sockets;
#endif
using System.Text;

namespace Roomie
{
    /// <summary>
    /// C# Roomie client — length-prefixed frames with bincode varint payload.
    /// Uses TCP on desktop/mobile and WebSocket on WebGL (via RoomieWebSocket.jslib).
    /// </summary>
    public class RoomieClient
    {
        private const int MaxFrameLen = 64 * 1024;

#if UNITY_WEBGL && !UNITY_EDITOR
        // ── WebSocket transport via JavaScript bridge ────────────────────
        [DllImport("__Internal")] private static extern void RoomieWS_Connect(string host, int port);
        [DllImport("__Internal")] private static extern int  RoomieWS_GetState();
        [DllImport("__Internal")] private static extern int  RoomieWS_Send(IntPtr data, int length);
        [DllImport("__Internal")] private static extern int  RoomieWS_HasData();
        [DllImport("__Internal")] private static extern int  RoomieWS_GetNextMessage(IntPtr buffer, int maxLen);
        [DllImport("__Internal")] private static extern void RoomieWS_Close();

        private IntPtr _wsRecvBuffer;
#else
        // ── TCP transport ────────────────────────────────────────────────
        private TcpClient _tcp;
        private NetworkStream _stream;
        private readonly byte[] _recvChunk = new byte[4096];
#endif

        private readonly List<byte> _recvBuf = new List<byte>();

        // Session state
        private string _roomCode = "";
        private uint _peerId;
        private uint _hostPeerId;
        private byte _side;
        private string _lastError = "";
        private bool _connected;

        // Reconnect info
        private string _lastHost;
        private int _lastPort;
        private int _lastWsPort;
        private string _lastAppKey;
        private string _lastClientHash;
        private string _lastDisplayName;
        private string _lastAuthToken;
        private string _lastPassword;
        private string _lastRoomCode;

        // Protocol handlers (wired to events)
        private ServerMessageDecoder.Handlers _handlers;

        // --- Events ---
        public event Action<JoinedArgs> OnJoined;
        public event Action<PeerJoinedArgs> OnPeerJoined;
        public event Action<uint> OnPeerLeft;
        public event Action<PeerInputArgs> OnPeerInput;
        public event Action<byte[]> OnStateUpdate;
        public event Action OnMoveAccepted;
        public event Action<string> OnMoveRejected;
        public event Action<uint> OnHostTransferred;
        public event Action OnKicked;
        public event Action OnBanned;
        public event Action<RoomSettings> OnSettingsChanged;
        public event Action OnPong;
        public event Action<string> OnError;
        public event Action<List<RoomListEntry>> OnRoomList;
        public event Action OnDisconnected;

        // --- Properties ---
        public string RoomCode => _roomCode;
        public uint PeerId => _peerId;
        public uint HostPeerId => _hostPeerId;
        public byte Side => _side;
        public bool IsHost => _peerId != 0 && _peerId == _hostPeerId;
        public string LastError => _lastError;

#if UNITY_WEBGL && !UNITY_EDITOR
        /// <summary>
        /// On WebGL, returns true while a connection attempt is active.
        /// The jslib buffers sends until the WebSocket handshake completes.
        /// </summary>
        public bool IsConnected => _connected;
#else
        public bool IsConnected => _connected && _tcp != null && _tcp.Connected;
#endif

        public RoomieClient()
        {
            WireHandlers();
        }

        private void WireHandlers()
        {
            _handlers = new ServerMessageDecoder.Handlers
            {
                OnJoined = args =>
                {
                    _roomCode = args.RoomCode;
                    _peerId = args.PeerId;
                    _hostPeerId = args.HostPeerId;
                    _side = args.Side;
                    OnJoined?.Invoke(args);
                },
                OnRoomList = list => OnRoomList?.Invoke(list),
                OnPeerJoined = args => OnPeerJoined?.Invoke(args),
                OnPeerLeft = peerId => OnPeerLeft?.Invoke(peerId),
                OnPeerInput = args => OnPeerInput?.Invoke(args),
                OnStateUpdate = data => OnStateUpdate?.Invoke(data),
                OnMoveAccepted = () => OnMoveAccepted?.Invoke(),
                OnMoveRejected = reason => OnMoveRejected?.Invoke(reason),
                OnHostTransferred = newHost =>
                {
                    _hostPeerId = newHost;
                    OnHostTransferred?.Invoke(newHost);
                },
                OnKicked = () => OnKicked?.Invoke(),
                OnBanned = () => OnBanned?.Invoke(),
                OnSettingsChanged = settings => OnSettingsChanged?.Invoke(settings),
                OnPong = () => OnPong?.Invoke(),
                OnError = msg =>
                {
                    _lastError = msg;
                    OnError?.Invoke(msg);
                }
            };
        }

        /// <summary>
        /// Connect to Roomie server.
        /// On desktop/mobile this uses TCP and blocks until connected.
        /// On WebGL this starts a WebSocket connection asynchronously
        /// (sends are buffered until the handshake completes).
        /// <param name="wsPort">WebSocket port (default 0 = tcpPort + 1).</param>
        /// </summary>
        public bool Connect(string host, int port, int wsPort = 0)
        {
            Disconnect();
            _lastHost = host;
            _lastPort = port;
            _lastWsPort = wsPort;

            try
            {
#if UNITY_WEBGL && !UNITY_EDITOR
                int actualWsPort = wsPort > 0 ? wsPort : port + 1;
                if (_wsRecvBuffer == IntPtr.Zero)
                    _wsRecvBuffer = Marshal.AllocHGlobal(MaxFrameLen);
                RoomieWS_Connect(host, actualWsPort);
                _connected = true;
                _recvBuf.Clear();
                return true;
#else
                _tcp = new TcpClient();
                _tcp.NoDelay = true;
                _tcp.Connect(host, port);
                _stream = _tcp.GetStream();
                _connected = true;
                _recvBuf.Clear();
                return true;
#endif
            }
            catch (Exception e)
            {
                _lastError = e.Message;
                _connected = false;
                return false;
            }
        }

        /// <summary>
        /// Disconnect and clear session state.
        /// </summary>
        public void Disconnect()
        {
#if UNITY_WEBGL && !UNITY_EDITOR
            RoomieWS_Close();
#else
            if (_stream != null)
            {
                try { _stream.Close(); } catch { }
                _stream = null;
            }
            if (_tcp != null)
            {
                try { _tcp.Close(); } catch { }
                _tcp = null;
            }
#endif
            bool wasConnected = _connected;
            _connected = false;
            _roomCode = "";
            _peerId = 0;
            _hostPeerId = 0;
            _side = 0;
            _recvBuf.Clear();

            if (wasConnected)
                OnDisconnected?.Invoke();
        }

        /// <summary>
        /// Create a new room. Sends Hello with empty room_code.
        /// </summary>
        public bool CreateRoom(string appKey, string clientHash,
            string displayName = null, string authToken = null, string password = null)
        {
            _lastAppKey = appKey;
            _lastClientHash = clientHash;
            _lastDisplayName = displayName;
            _lastAuthToken = authToken;
            _lastPassword = password;
            _lastRoomCode = "";
            return SendHello(appKey, "", clientHash, displayName, authToken, password);
        }

        /// <summary>
        /// Join existing room by code.
        /// </summary>
        public bool JoinRoom(string appKey, string roomCode, string clientHash,
            string displayName = null, string authToken = null, string password = null)
        {
            _lastAppKey = appKey;
            _lastClientHash = clientHash;
            _lastDisplayName = displayName;
            _lastAuthToken = authToken;
            _lastPassword = password;
            _lastRoomCode = roomCode;
            return SendHello(appKey, roomCode, clientHash, displayName, authToken, password);
        }

        /// <summary>
        /// Attempt to reconnect to the last room using the stored room code.
        /// </summary>
        public bool Reconnect()
        {
            if (string.IsNullOrEmpty(_lastHost) || string.IsNullOrEmpty(_lastRoomCode))
                return false;

            if (!Connect(_lastHost, _lastPort, _lastWsPort))
                return false;

            return JoinRoom(_lastAppKey, _lastRoomCode, _lastClientHash,
                _lastDisplayName, _lastAuthToken, _lastPassword);
        }

        public bool SendMove(byte[] data) => SendPayload(ClientMessageEncoder.EncodeMove(data));
        public bool SendInput(byte[] data) => SendPayload(ClientMessageEncoder.EncodeInput(data));
        public bool RequestState() => SendPayload(ClientMessageEncoder.EncodeRequestState());
        public bool SendPing() => SendPayload(ClientMessageEncoder.EncodePing());
        public bool ListRooms(string appKey) => SendPayload(ClientMessageEncoder.EncodeListRooms(appKey));
        public bool SetRoomPublic(bool isPublic) => SendPayload(ClientMessageEncoder.EncodeSetPublic(isPublic));
        public bool LeaveRoom()
        {
            bool ok = SendPayload(ClientMessageEncoder.EncodeLeave());
            _roomCode = "";
            _peerId = 0;
            _hostPeerId = 0;
            return ok;
        }
        public bool KickPeer(uint peerId) => SendPayload(ClientMessageEncoder.EncodeKick(peerId));
        public bool BanPeer(uint peerId) => SendPayload(ClientMessageEncoder.EncodeBan(peerId));
        public bool TransferHost(uint peerId) => SendPayload(ClientMessageEncoder.EncodeTransferHost(peerId));
        public bool AcceptPeer(uint peerId) => SendPayload(ClientMessageEncoder.EncodeAcceptPeer(peerId));
        public bool DenyPeer(uint peerId) => SendPayload(ClientMessageEncoder.EncodeDenyPeer(peerId));

        /// <summary>
        /// Non-blocking poll. Reads available data and dispatches events.
        /// Call every frame.
        /// </summary>
        public void Poll()
        {
            if (!_connected) return;

            try
            {
#if UNITY_WEBGL && !UNITY_EDITOR
                // Check WebSocket state
                int wsState = RoomieWS_GetState();
                if (wsState == 0 || wsState == 3) // closed or error
                {
                    HandleDisconnect();
                    return;
                }
                if (wsState == 1) return; // still connecting

                // Read all available messages from the JS receive buffer
                while (RoomieWS_HasData() == 1)
                {
                    int len = RoomieWS_GetNextMessage(_wsRecvBuffer, MaxFrameLen);
                    if (len <= 0) break;
                    for (int i = 0; i < len; i++)
                        _recvBuf.Add(Marshal.ReadByte(_wsRecvBuffer, i));
                }
#else
                if (_tcp == null || !_tcp.Connected)
                {
                    HandleDisconnect();
                    return;
                }

                // Read whatever is available
                while (_stream.DataAvailable)
                {
                    int n = _stream.Read(_recvChunk, 0, _recvChunk.Length);
                    if (n <= 0)
                    {
                        HandleDisconnect();
                        return;
                    }
                    for (int i = 0; i < n; i++)
                        _recvBuf.Add(_recvChunk[i]);
                }
#endif

                // Process complete frames (identical for both transports)
                while (TryProcessFrame()) { }
            }
            catch (Exception e)
            {
                _lastError = e.Message;
                HandleDisconnect();
            }
        }

        // --- Private helpers ---

        private bool SendHello(string appKey, string roomCode, string clientHash,
            string displayName, string authToken, string password)
        {
            return SendPayload(ClientMessageEncoder.EncodeHello(
                appKey, roomCode, clientHash, displayName, authToken, password));
        }

        private bool SendPayload(byte[] payload)
        {
            if (!_connected)
            {
                _lastError = "Not connected";
                return false;
            }
            if (payload.Length > MaxFrameLen)
            {
                _lastError = "Payload too large";
                return false;
            }

            try
            {
                // 4-byte LE length prefix + payload
                byte[] frame = new byte[4 + payload.Length];
                int len = payload.Length;
                frame[0] = (byte)len;
                frame[1] = (byte)(len >> 8);
                frame[2] = (byte)(len >> 16);
                frame[3] = (byte)(len >> 24);
                Array.Copy(payload, 0, frame, 4, payload.Length);

#if UNITY_WEBGL && !UNITY_EDITOR
                IntPtr framePtr = Marshal.AllocHGlobal(frame.Length);
                Marshal.Copy(frame, 0, framePtr, frame.Length);
                RoomieWS_Send(framePtr, frame.Length);
                Marshal.FreeHGlobal(framePtr);
#else
                _stream.Write(frame, 0, frame.Length);
                _stream.Flush();
#endif
                return true;
            }
            catch (Exception e)
            {
                _lastError = e.Message;
                HandleDisconnect();
                return false;
            }
        }

        private bool TryProcessFrame()
        {
            if (_recvBuf.Count < 4) return false;

            uint len = (uint)_recvBuf[0]
                | ((uint)_recvBuf[1] << 8)
                | ((uint)_recvBuf[2] << 16)
                | ((uint)_recvBuf[3] << 24);

            if (len > MaxFrameLen)
            {
                _lastError = "Frame too large";
                HandleDisconnect();
                return false;
            }

            if (_recvBuf.Count < 4 + (int)len) return false;

            // Extract payload
            byte[] payload = new byte[len];
            _recvBuf.CopyTo(4, payload, 0, (int)len);
            _recvBuf.RemoveRange(0, 4 + (int)len);

            // Decode and dispatch
            try
            {
                ServerMessageDecoder.DecodeAndDispatch(payload, 0, payload.Length, ref _handlers);
            }
            catch (ProtocolException e)
            {
                _lastError = "Decode error: " + e.Message;
                OnError?.Invoke(_lastError);
            }

            return true;
        }

        private void HandleDisconnect()
        {
#if UNITY_WEBGL && !UNITY_EDITOR
            RoomieWS_Close();
#else
            if (_stream != null)
            {
                try { _stream.Close(); } catch { }
                _stream = null;
            }
            if (_tcp != null)
            {
                try { _tcp.Close(); } catch { }
                _tcp = null;
            }
#endif
            _connected = false;
            _recvBuf.Clear();
            OnDisconnected?.Invoke();
        }
    }
}
