// WebSocket transport for the Roomie C# client in Unity WebGL builds.
// Wraps the browser WebSocket API and exposes it via DllImport("__Internal").
// The same 4-byte LE length-prefixed frame protocol is used over WebSocket
// (the Roomie server expects identical framing for both TCP and WS).

var RoomieWebSocketPlugin = {

    $wsState: {
        socket: null,
        connected: false,
        receiveBuffer: [],   // Array of Uint8Array messages
        sendQueue: [],       // Buffered sends before connection opens
        error: null
    },

    /**
     * Open a WebSocket connection. Automatically selects ws:// or wss://
     * based on the hosting page's protocol (so HTTPS pages use wss://).
     * Queued sends are flushed once the connection opens.
     */
    RoomieWS_Connect: function (hostPtr, port) {
        // Clean up any previous connection
        if (wsState.socket) {
            try { wsState.socket.close(); } catch (e) {}
            wsState.socket = null;
        }
        wsState.connected = false;
        wsState.error = null;
        wsState.receiveBuffer = [];
        wsState.sendQueue = [];

        var host = UTF8ToString(hostPtr);
        var protocol = (location.protocol === 'https:') ? 'wss://' : 'ws://';
        var url = protocol + host + ':' + port;

        console.log('[RoomieWS] Connecting to ' + url);

        var ws = new WebSocket(url);
        ws.binaryType = 'arraybuffer';
        wsState.socket = ws;

        ws.onopen = function () {
            console.log('[RoomieWS] Connected');
            wsState.connected = true;
            // Flush queued sends
            for (var i = 0; i < wsState.sendQueue.length; i++) {
                ws.send(wsState.sendQueue[i]);
            }
            wsState.sendQueue = [];
        };

        ws.onmessage = function (event) {
            wsState.receiveBuffer.push(new Uint8Array(event.data));
        };

        ws.onerror = function () {
            console.error('[RoomieWS] WebSocket error');
            wsState.error = 'WebSocket error';
        };

        ws.onclose = function (event) {
            console.log('[RoomieWS] Closed (code=' + event.code + ')');
            wsState.connected = false;
        };
    },

    /**
     * Returns the connection state:
     *   0 = closed / not started
     *   1 = connecting (handshake in progress)
     *   2 = open (ready for data)
     *   3 = error
     */
    RoomieWS_GetState: function () {
        if (wsState.error) return 3;
        if (!wsState.socket) return 0;
        switch (wsState.socket.readyState) {
            case 0: return 1; // CONNECTING
            case 1: return 2; // OPEN
            default: return 0; // CLOSING / CLOSED
        }
    },

    /**
     * Send a binary frame. If the socket is still connecting, the data is
     * queued and will be flushed automatically when the connection opens.
     * dataPtr/length point into the Emscripten heap.
     */
    RoomieWS_Send: function (dataPtr, length) {
        if (!wsState.socket) return 0;
        // Copy from Emscripten heap (the source buffer may be freed)
        var data = new Uint8Array(HEAPU8.buffer, dataPtr, length).slice();
        if (wsState.connected && wsState.socket.readyState === 1) {
            wsState.socket.send(data.buffer);
        } else {
            wsState.sendQueue.push(data.buffer);
        }
        return 1;
    },

    /** Returns 1 if there are buffered messages to read. */
    RoomieWS_HasData: function () {
        return wsState.receiveBuffer.length > 0 ? 1 : 0;
    },

    /**
     * Copies the next received message into bufferPtr (up to maxLen bytes).
     * Returns the number of bytes written, or 0 if no messages are available.
     */
    RoomieWS_GetNextMessage: function (bufferPtr, maxLen) {
        if (wsState.receiveBuffer.length === 0) return 0;
        var msg = wsState.receiveBuffer.shift();
        var len = Math.min(msg.length, maxLen);
        HEAPU8.set(msg.subarray(0, len), bufferPtr);
        return len;
    },

    /** Close the WebSocket and clear all state. */
    RoomieWS_Close: function () {
        if (wsState.socket) {
            try { wsState.socket.close(); } catch (e) {}
            wsState.socket = null;
        }
        wsState.connected = false;
        wsState.error = null;
        wsState.receiveBuffer = [];
        wsState.sendQueue = [];
    }
};

autoAddDeps(RoomieWebSocketPlugin, '$wsState');
mergeInto(LibraryManager.library, RoomieWebSocketPlugin);
