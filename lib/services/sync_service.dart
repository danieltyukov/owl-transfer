import 'dart:async';
import 'dart:typed_data';
import '../models/device.dart';
import '../models/sync_file.dart';
import 'file_service.dart';
import 'p2p_service.dart';
import 'settings_service.dart';

enum SyncStatus {
  idle,
  syncing,
  complete,
  error,
}

class SyncService {
  final FileService _fileService;
  final P2PService _p2pService;
  final SettingsService _settingsService;

  Timer? _syncTimer;
  SyncStatus _status = SyncStatus.idle;
  String? _lastError;
  DateTime? _lastSync;

  final _statusController = StreamController<SyncStatus>.broadcast();
  final _progressController = StreamController<double>.broadcast();

  Stream<SyncStatus> get statusStream => _statusController.stream;
  Stream<double> get progressStream => _progressController.stream;
  SyncStatus get status => _status;
  String? get lastError => _lastError;
  DateTime? get lastSync => _lastSync;

  SyncService(this._fileService, this._p2pService, this._settingsService) {
    _setupListeners();
  }

  void _setupListeners() {
    _p2pService.messages.listen((event) async {
      final (deviceId, message) = event;

      switch (message.type) {
        case MessageType.manifest:
          await _handleManifest(deviceId, message);
          break;
        case MessageType.fileRequest:
          await _handleFileRequest(deviceId, message);
          break;
        case MessageType.syncComplete:
          _setStatus(SyncStatus.complete);
          break;
        default:
          break;
      }
    });

    _p2pService.fileData.listen((event) async {
      final (_, filePath, data) = event;
      await _receiveFile(filePath, data);
    });
  }

  void _setStatus(SyncStatus status) {
    _status = status;
    _statusController.add(status);
  }

  Future<void> startAutoSync() async {
    final interval = await _settingsService.getSyncInterval();

    _syncTimer?.cancel();
    _syncTimer = Timer.periodic(Duration(minutes: interval), (_) {
      syncWithAllDevices();
    });
  }

  void stopAutoSync() {
    _syncTimer?.cancel();
    _syncTimer = null;
  }

  Future<void> updateSyncInterval() async {
    stopAutoSync();
    await startAutoSync();
  }

  Future<void> syncWithAllDevices() async {
    final devices = await _settingsService.getPairedDevices();

    for (final device in devices) {
      if (_p2pService.isConnected(device.id)) {
        await syncWithDevice(device.id);
      }
    }
  }

  Future<void> syncWithDevice(String deviceId) async {
    if (_status == SyncStatus.syncing) return;

    _setStatus(SyncStatus.syncing);
    _progressController.add(0.0);

    try {
      // Generate and send our manifest
      final manifest = await _fileService.generateManifest(
        _p2pService.deviceId,
        _p2pService.deviceName,
      );

      _p2pService.sendMessage(deviceId, P2PMessage(
        type: MessageType.manifest,
        data: manifest.toJson(),
      ));

      _progressController.add(0.1);
    } catch (e) {
      _lastError = e.toString();
      _setStatus(SyncStatus.error);
    }
  }

  Future<void> _handleManifest(String deviceId, P2PMessage message) async {
    try {
      final remoteManifest = SyncManifest.fromJson(message.data);
      final localFiles = await _fileService.listAllFilesRecursive();

      // Find files we need to request
      final filesToRequest = <SyncFile>[];

      for (final remoteFile in remoteManifest.files) {
        if (remoteFile.isDirectory) continue;

        final localFile = localFiles.firstWhere(
          (f) => f.relativePath == remoteFile.relativePath,
          orElse: () => SyncFile(
            id: '',
            name: '',
            relativePath: '',
            isDirectory: false,
            size: 0,
            modifiedAt: DateTime.fromMillisecondsSinceEpoch(0),
          ),
        );

        // Request if file doesn't exist locally or has different checksum
        if (localFile.id.isEmpty || localFile.checksum != remoteFile.checksum) {
          filesToRequest.add(remoteFile);
        }
      }

      // Find files we need to send
      final filesToSend = <SyncFile>[];

      for (final localFile in localFiles) {
        if (localFile.isDirectory) continue;

        final remoteFile = remoteManifest.files.firstWhere(
          (f) => f.relativePath == localFile.relativePath,
          orElse: () => SyncFile(
            id: '',
            name: '',
            relativePath: '',
            isDirectory: false,
            size: 0,
            modifiedAt: DateTime.fromMillisecondsSinceEpoch(0),
          ),
        );

        // Send if file doesn't exist remotely or we have newer version
        if (remoteFile.id.isEmpty ||
            (localFile.checksum != remoteFile.checksum &&
             localFile.modifiedAt.isAfter(remoteFile.modifiedAt))) {
          filesToSend.add(localFile);
        }
      }

      // Create directories first
      for (final remoteFile in remoteManifest.files) {
        if (remoteFile.isDirectory) {
          await _fileService.createFolder(remoteFile.name, parentPath: _getParentPath(remoteFile.relativePath));
        }
      }

      // Request needed files
      for (final file in filesToRequest) {
        _p2pService.sendMessage(deviceId, P2PMessage(
          type: MessageType.fileRequest,
          data: {'relativePath': file.relativePath},
        ));
      }

      // Send our files
      final totalToSend = filesToSend.length;
      for (int i = 0; i < filesToSend.length; i++) {
        final file = filesToSend[i];
        final bytes = await _fileService.readFileBytes(file.relativePath);
        _p2pService.sendFile(deviceId, file.relativePath, Uint8List.fromList(bytes));
        _progressController.add(0.1 + (0.8 * (i + 1) / totalToSend));
      }

      if (filesToRequest.isEmpty) {
        _p2pService.sendMessage(deviceId, P2PMessage(
          type: MessageType.syncComplete,
          data: {},
        ));
        _lastSync = DateTime.now();
        _setStatus(SyncStatus.complete);
      }

      _progressController.add(1.0);
    } catch (e) {
      _lastError = e.toString();
      _setStatus(SyncStatus.error);
    }
  }

  Future<void> _handleFileRequest(String deviceId, P2PMessage message) async {
    try {
      final relativePath = message.data['relativePath'] as String;
      final bytes = await _fileService.readFileBytes(relativePath);
      _p2pService.sendFile(deviceId, relativePath, Uint8List.fromList(bytes));
    } catch (e) {
      print('Error handling file request: $e');
    }
  }

  Future<void> _receiveFile(String filePath, Uint8List data) async {
    try {
      final fileName = filePath.split('/').last;
      final parentPath = _getParentPath(filePath);

      await _fileService.addFileFromBytes(fileName, data, targetPath: parentPath.isEmpty ? null : parentPath);

      _lastSync = DateTime.now();
    } catch (e) {
      print('Error receiving file: $e');
    }
  }

  String _getParentPath(String path) {
    final parts = path.split('/');
    if (parts.length <= 1) return '';
    return parts.sublist(0, parts.length - 1).join('/');
  }

  void dispose() {
    _syncTimer?.cancel();
    _statusController.close();
    _progressController.close();
  }
}
