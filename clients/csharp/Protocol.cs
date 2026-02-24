using System;
using System.Collections.Generic;

namespace Roomie
{
    // --- Structs ---

    public struct RoomSettings
    {
        public uint MaxPlayers;
        public uint MinPlayers;
        public bool IsPublic;
        public string Password; // null = None
        public bool AcceptRequired;
        // custom: always skip on read, write None

        public static RoomSettings Decode(BincodeReader r)
        {
            var s = new RoomSettings();
            s.MaxPlayers = r.ReadU32Varint();
            s.MinPlayers = r.ReadU32Varint();
            s.IsPublic = r.ReadBool();
            s.Password = r.ReadOptString();
            s.AcceptRequired = r.ReadBool();
            byte customTag = r.ReadU8();
            if (customTag != 0) r.SkipBincodeJsonValue();
            return s;
        }

        public static void Encode(BincodeWriter w, RoomSettings s)
        {
            w.WriteVarint(s.MaxPlayers);
            w.WriteVarint(s.MinPlayers);
            w.WriteBool(s.IsPublic);
            w.WriteOptString(s.Password);
            w.WriteBool(s.AcceptRequired);
            w.WriteU8(0); // custom = None
        }
    }

    public struct RoomListEntry
    {
        public string RoomCode;
        public uint PlayerCount;
        public RoomSettings Settings;
    }

    // --- Enums ---

    public enum RoomControlType : byte
    {
        CreateRoom = 0,
        UpdateSettings = 1,
        SetPublic = 2,
        AcceptPeer = 3,
        DenyPeer = 4,
        Kick = 5,
        Ban = 6,
        TransferHost = 7,
        Leave = 8
    }

    public enum RoomEventType : byte
    {
        HostTransferred = 0,
        Kicked = 1,
        Banned = 2,
        SettingsChanged = 3
    }

    // --- Server Message Event Args ---

    public struct JoinedArgs
    {
        public string RoomCode;
        public uint PeerId;
        public uint HostPeerId;
        public byte Side;
        public RoomSettings Settings;
        public byte[] InitialState; // null if None
    }

    public struct PeerJoinedArgs
    {
        public uint PeerId;
        public string ClientHash;
        public string DisplayName; // null if None
    }

    public struct PeerInputArgs
    {
        public uint PeerId;
        public byte[] Data;
    }

    // --- Encode ClientMessage ---

    public static class ClientMessageEncoder
    {
        /// <summary>Hello (variant 0). Empty roomCode = create room.</summary>
        public static byte[] EncodeHello(string appKey, string roomCode, string clientHash,
            string displayName = null, string authToken = null, string password = null)
        {
            var w = new BincodeWriter();
            w.WriteVarint(0); // Hello
            w.WriteString(appKey);
            w.WriteString(roomCode);
            w.WriteString(clientHash);
            w.WriteOptString(displayName);
            w.WriteOptString(authToken);
            w.WriteOptString(password);
            return w.ToArray();
        }

        /// <summary>Move (variant 1). State blob, host only.</summary>
        public static byte[] EncodeMove(byte[] data)
        {
            var w = new BincodeWriter();
            w.WriteVarint(1);
            w.WriteBytes(data);
            return w.ToArray();
        }

        /// <summary>Input (variant 2). Input blob, forwarded to host as PeerInput.</summary>
        public static byte[] EncodeInput(byte[] data)
        {
            var w = new BincodeWriter();
            w.WriteVarint(2);
            w.WriteBytes(data);
            return w.ToArray();
        }

        /// <summary>RoomControl (variant 3) + CreateRoom (sub-variant 0).</summary>
        public static byte[] EncodeCreateRoom(RoomSettings settings)
        {
            var w = new BincodeWriter();
            w.WriteVarint(3); // RoomControl
            w.WriteVarint(0); // CreateRoom
            RoomSettings.Encode(w, settings);
            return w.ToArray();
        }

        /// <summary>RoomControl (variant 3) + UpdateSettings (sub-variant 1).</summary>
        public static byte[] EncodeUpdateSettings(RoomSettings settings)
        {
            var w = new BincodeWriter();
            w.WriteVarint(3);
            w.WriteVarint(1);
            RoomSettings.Encode(w, settings);
            return w.ToArray();
        }

        /// <summary>RoomControl (variant 3) + SetPublic (sub-variant 2).</summary>
        public static byte[] EncodeSetPublic(bool isPublic)
        {
            var w = new BincodeWriter();
            w.WriteVarint(3);
            w.WriteVarint(2);
            w.WriteBool(isPublic);
            return w.ToArray();
        }

        /// <summary>RoomControl (variant 3) + AcceptPeer (sub-variant 3).</summary>
        public static byte[] EncodeAcceptPeer(uint peerId)
        {
            var w = new BincodeWriter();
            w.WriteVarint(3);
            w.WriteVarint(3);
            w.WriteVarint(peerId);
            return w.ToArray();
        }

        /// <summary>RoomControl (variant 3) + DenyPeer (sub-variant 4).</summary>
        public static byte[] EncodeDenyPeer(uint peerId)
        {
            var w = new BincodeWriter();
            w.WriteVarint(3);
            w.WriteVarint(4);
            w.WriteVarint(peerId);
            return w.ToArray();
        }

        /// <summary>RoomControl (variant 3) + Kick (sub-variant 5).</summary>
        public static byte[] EncodeKick(uint peerId)
        {
            var w = new BincodeWriter();
            w.WriteVarint(3);
            w.WriteVarint(5);
            w.WriteVarint(peerId);
            return w.ToArray();
        }

        /// <summary>RoomControl (variant 3) + Ban (sub-variant 6).</summary>
        public static byte[] EncodeBan(uint peerId)
        {
            var w = new BincodeWriter();
            w.WriteVarint(3);
            w.WriteVarint(6);
            w.WriteVarint(peerId);
            return w.ToArray();
        }

        /// <summary>RoomControl (variant 3) + TransferHost (sub-variant 7).</summary>
        public static byte[] EncodeTransferHost(uint peerId)
        {
            var w = new BincodeWriter();
            w.WriteVarint(3);
            w.WriteVarint(7);
            w.WriteVarint(peerId);
            return w.ToArray();
        }

        /// <summary>RoomControl (variant 3) + Leave (sub-variant 8).</summary>
        public static byte[] EncodeLeave()
        {
            var w = new BincodeWriter();
            w.WriteVarint(3);
            w.WriteVarint(8);
            return w.ToArray();
        }

        /// <summary>RequestState (variant 4).</summary>
        public static byte[] EncodeRequestState()
        {
            var w = new BincodeWriter();
            w.WriteVarint(4);
            return w.ToArray();
        }

        /// <summary>Ping (variant 5).</summary>
        public static byte[] EncodePing()
        {
            var w = new BincodeWriter();
            w.WriteVarint(5);
            return w.ToArray();
        }

        /// <summary>ListRooms (variant 6).</summary>
        public static byte[] EncodeListRooms(string appKey)
        {
            var w = new BincodeWriter();
            w.WriteVarint(6);
            w.WriteString(appKey);
            return w.ToArray();
        }
    }

    // --- Decode ServerMessage ---

    public static class ServerMessageDecoder
    {
        public delegate void JoinedHandler(JoinedArgs args);
        public delegate void RoomListHandler(List<RoomListEntry> rooms);
        public delegate void PeerJoinedHandler(PeerJoinedArgs args);
        public delegate void PeerLeftHandler(uint peerId);
        public delegate void PeerInputHandler(PeerInputArgs args);
        public delegate void StateUpdateHandler(byte[] data);
        public delegate void MoveAcceptedHandler();
        public delegate void MoveRejectedHandler(string reason);
        public delegate void HostTransferredHandler(uint newHostPeerId);
        public delegate void KickedHandler();
        public delegate void BannedHandler();
        public delegate void SettingsChangedHandler(RoomSettings settings);
        public delegate void PongHandler();
        public delegate void ErrorHandler(string message);

        public struct Handlers
        {
            public JoinedHandler OnJoined;
            public RoomListHandler OnRoomList;
            public PeerJoinedHandler OnPeerJoined;
            public PeerLeftHandler OnPeerLeft;
            public PeerInputHandler OnPeerInput;
            public StateUpdateHandler OnStateUpdate;
            public MoveAcceptedHandler OnMoveAccepted;
            public MoveRejectedHandler OnMoveRejected;
            public HostTransferredHandler OnHostTransferred;
            public KickedHandler OnKicked;
            public BannedHandler OnBanned;
            public SettingsChangedHandler OnSettingsChanged;
            public PongHandler OnPong;
            public ErrorHandler OnError;
        }

        /// <summary>
        /// Decode a single ServerMessage from payload and dispatch to handlers.
        /// </summary>
        public static void DecodeAndDispatch(byte[] payload, int offset, int length, ref Handlers handlers)
        {
            var r = new BincodeReader(payload, offset, length);
            ulong tag = r.ReadVarint();

            switch (tag)
            {
                case 0: // Joined
                {
                    var args = new JoinedArgs();
                    args.RoomCode = r.ReadString();
                    args.PeerId = r.ReadU32Varint();
                    args.HostPeerId = r.ReadU32Varint();
                    args.Side = r.ReadU8();
                    args.Settings = RoomSettings.Decode(r);
                    byte hasState = r.ReadU8();
                    args.InitialState = hasState != 0 ? r.ReadBytes() : null;
                    handlers.OnJoined?.Invoke(args);
                    break;
                }
                case 1: // RoomList
                {
                    ulong n = r.ReadVarint();
                    var list = new List<RoomListEntry>();
                    for (ulong i = 0; i < n && !r.Eof; i++)
                    {
                        var entry = new RoomListEntry();
                        entry.RoomCode = r.ReadString();
                        entry.PlayerCount = r.ReadU32Varint();
                        entry.Settings = RoomSettings.Decode(r);
                        list.Add(entry);
                    }
                    handlers.OnRoomList?.Invoke(list);
                    break;
                }
                case 2: // PeerJoined
                {
                    var args = new PeerJoinedArgs();
                    args.PeerId = r.ReadU32Varint();
                    args.ClientHash = r.ReadString();
                    args.DisplayName = r.ReadOptString();
                    handlers.OnPeerJoined?.Invoke(args);
                    break;
                }
                case 3: // PeerLeft
                {
                    uint peerId = r.ReadU32Varint();
                    handlers.OnPeerLeft?.Invoke(peerId);
                    break;
                }
                case 4: // PeerInput
                {
                    var args = new PeerInputArgs();
                    args.PeerId = r.ReadU32Varint();
                    args.Data = r.ReadBytes();
                    handlers.OnPeerInput?.Invoke(args);
                    break;
                }
                case 5: // StateUpdate
                {
                    byte[] data = r.ReadBytes();
                    handlers.OnStateUpdate?.Invoke(data);
                    break;
                }
                case 6: // MoveAccepted
                    handlers.OnMoveAccepted?.Invoke();
                    break;
                case 7: // MoveRejected
                {
                    string reason = r.ReadString();
                    handlers.OnMoveRejected?.Invoke(reason);
                    break;
                }
                case 8: // RoomEvent
                {
                    ulong ev = r.ReadVarint();
                    switch (ev)
                    {
                        case 0: // HostTransferred
                            uint newHost = r.ReadU32Varint();
                            handlers.OnHostTransferred?.Invoke(newHost);
                            break;
                        case 1: // Kicked
                            handlers.OnKicked?.Invoke();
                            break;
                        case 2: // Banned
                            handlers.OnBanned?.Invoke();
                            break;
                        case 3: // SettingsChanged
                            var settings = RoomSettings.Decode(r);
                            handlers.OnSettingsChanged?.Invoke(settings);
                            break;
                    }
                    break;
                }
                case 9: // Pong
                    handlers.OnPong?.Invoke();
                    break;
                case 10: // Error
                {
                    string msg = r.ReadString();
                    handlers.OnError?.Invoke(msg);
                    break;
                }
            }
        }
    }
}
