import 'dart:async';
import 'dart:convert';
import 'dart:io';
import 'dart:typed_data';
import 'package:uuid/uuid.dart';
import '../models/device.dart';
import '../models/sync_file.dart';

enum MessageType {
  ping,
  pong,
  pairRequest,
  pairAccept,
  pairReject,
  manifest,
  fileRequest,
  fileData,
  syncComplete,
}

class P2PMessage {
  final MessageType type;
  final Map<String, dynamic> data;

  P2PMessage({required this.type, required this.data});

  Map<String, dynamic> toJson() => {
        'type': type.index,
        'data': data,
      };

  factory P2PMessage.fromJson(Map<String, dynamic> json) => P2PMessage(
        type: MessageType.values[json['type']],
        data: json['data'],
      );

  String toJsonString() => jsonEncode(toJson());

  factory P2PMessage.fromJsonString(String json) =>
      P2PMessage.fromJson(jsonDecode(json));
}

class P2PService {
  static const int defaultPort = 52734;

  ServerSocket? _server;
  final Map<String, Socket> _connections = {};
  final _uuid = const Uuid();

  late String deviceId;
  late String deviceName;
  String? _localIp;
  String? _pairingCode;

  final _messageController = StreamController<(String, P2PMessage)>.broadcast();
  final _fileDataController = StreamController<(String, String, Uint8List)>.broadcast();
  final _connectionController = StreamController<(String, bool)>.broadcast();

  Stream<(String, P2PMessage)> get messages => _messageController.stream;
  Stream<(String, String, Uint8List)> get fileData => _fileDataController.stream;
  Stream<(String, bool)> get connectionChanges => _connectionController.stream;

  Future<void> initialize(String name) async {
    deviceId = _uuid.v4();
    deviceName = name;
    await _getLocalIp();
  }

  Future<void> _getLocalIp() async {
    try {
      final interfaces = await NetworkInterface.list(
        type: InternetAddressType.IPv4,
        includeLinkLocal: false,
      );

      for (final interface in interfaces) {
        for (final addr in interface.addresses) {
          if (!addr.isLoopback && addr.address.startsWith('192.168')) {
            _localIp = addr.address;
            return;
          }
        }
      }

      // Fallback to first non-loopback address
      for (final interface in interfaces) {
        for (final addr in interface.addresses) {
          if (!addr.isLoopback) {
            _localIp = addr.address;
            return;
          }
        }
      }
    } catch (e) {
      print('Error getting local IP: $e');
    }
  }

  String? get localIp => _localIp;

  Future<void> startServer() async {
    if (_server != null) return;

    try {
      _server = await ServerSocket.bind(InternetAddress.anyIPv4, defaultPort);
      print('Server started on ${_localIp}:$defaultPort');

      _server!.listen(_handleConnection);
    } catch (e) {
      print('Error starting server: $e');
      rethrow;
    }
  }

  void _handleConnection(Socket socket) {
    final remoteId = '${socket.remoteAddress.address}:${socket.remotePort}';
    print('New connection from: $remoteId');

    StringBuffer buffer = StringBuffer();

    socket.listen(
      (data) {
        buffer.write(utf8.decode(data));
        _processBuffer(buffer, remoteId, socket);
      },
      onError: (error) {
        print('Socket error: $error');
        _removeConnection(remoteId);
      },
      onDone: () {
        print('Connection closed: $remoteId');
        _removeConnection(remoteId);
      },
    );
  }

  void _processBuffer(StringBuffer buffer, String remoteId, Socket socket) {
    final content = buffer.toString();
    final lines = content.split('\n');

    for (int i = 0; i < lines.length - 1; i++) {
      final line = lines[i].trim();
      if (line.isNotEmpty) {
        try {
          // Check if it's binary file data (base64 encoded)
          if (line.startsWith('FILE:')) {
            final parts = line.substring(5).split(':');
            final filePath = parts[0];
            final base64Data = parts.sublist(1).join(':');
            final bytes = base64Decode(base64Data);
            _fileDataController.add((remoteId, filePath, Uint8List.fromList(bytes)));
          } else {
            final message = P2PMessage.fromJsonString(line);
            _handleMessage(remoteId, socket, message);
          }
        } catch (e) {
          print('Error parsing message: $e');
        }
      }
    }

    // Keep the incomplete line in the buffer
    buffer.clear();
    if (lines.isNotEmpty) {
      buffer.write(lines.last);
    }
  }

  void _handleMessage(String remoteId, Socket socket, P2PMessage message) {
    switch (message.type) {
      case MessageType.pairRequest:
        final code = message.data['pairingCode'];
        if (code == _pairingCode) {
          _connections[message.data['deviceId']] = socket;
          sendMessage(message.data['deviceId'], P2PMessage(
            type: MessageType.pairAccept,
            data: {'deviceId': deviceId, 'deviceName': deviceName},
          ));
          _connectionController.add((message.data['deviceId'], true));
        } else {
          sendMessageToSocket(socket, P2PMessage(
            type: MessageType.pairReject,
            data: {'reason': 'Invalid pairing code'},
          ));
        }
        break;

      case MessageType.pairAccept:
        _connections[message.data['deviceId']] = socket;
        _connectionController.add((message.data['deviceId'], true));
        break;

      case MessageType.ping:
        sendMessageToSocket(socket, P2PMessage(
          type: MessageType.pong,
          data: {'deviceId': deviceId},
        ));
        break;

      default:
        _messageController.add((remoteId, message));
    }
  }

  void _removeConnection(String id) {
    _connections.remove(id);
    _connectionController.add((id, false));
  }

  PairingInfo generatePairingInfo() {
    _pairingCode = _uuid.v4().substring(0, 8);
    return PairingInfo(
      deviceId: deviceId,
      deviceName: deviceName,
      ipAddress: _localIp ?? 'unknown',
      port: defaultPort,
      pairingCode: _pairingCode!,
    );
  }

  Future<bool> connectToPeer(PairingInfo info) async {
    try {
      final socket = await Socket.connect(info.ipAddress, info.port);
      _connections[info.deviceId] = socket;

      StringBuffer buffer = StringBuffer();
      socket.listen(
        (data) {
          buffer.write(utf8.decode(data));
          _processBuffer(buffer, info.deviceId, socket);
        },
        onError: (error) {
          print('Socket error: $error');
          _removeConnection(info.deviceId);
        },
        onDone: () {
          print('Connection closed: ${info.deviceId}');
          _removeConnection(info.deviceId);
        },
      );

      // Send pairing request
      sendMessage(info.deviceId, P2PMessage(
        type: MessageType.pairRequest,
        data: {
          'deviceId': deviceId,
          'deviceName': deviceName,
          'pairingCode': info.pairingCode,
        },
      ));

      return true;
    } catch (e) {
      print('Error connecting to peer: $e');
      return false;
    }
  }

  void sendMessage(String deviceId, P2PMessage message) {
    final socket = _connections[deviceId];
    if (socket != null) {
      sendMessageToSocket(socket, message);
    }
  }

  void sendMessageToSocket(Socket socket, P2PMessage message) {
    socket.writeln(message.toJsonString());
  }

  void sendFile(String deviceId, String filePath, Uint8List data) {
    final socket = _connections[deviceId];
    if (socket != null) {
      final base64Data = base64Encode(data);
      socket.writeln('FILE:$filePath:$base64Data');
    }
  }

  bool isConnected(String deviceId) {
    return _connections.containsKey(deviceId);
  }

  Future<bool> pingDevice(String deviceId) async {
    if (!_connections.containsKey(deviceId)) return false;

    try {
      sendMessage(deviceId, P2PMessage(
        type: MessageType.ping,
        data: {},
      ));
      return true;
    } catch (e) {
      return false;
    }
  }

  void disconnect(String deviceId) {
    final socket = _connections[deviceId];
    socket?.close();
    _connections.remove(deviceId);
  }

  Future<void> dispose() async {
    for (final socket in _connections.values) {
      await socket.close();
    }
    _connections.clear();
    await _server?.close();
    _server = null;
    await _messageController.close();
    await _fileDataController.close();
    await _connectionController.close();
  }
}
