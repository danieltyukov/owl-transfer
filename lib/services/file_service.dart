import 'dart:io';
import 'package:path_provider/path_provider.dart';
import '../models/sync_file.dart';

class FileService {
  late Directory _syncDirectory;
  bool _initialized = false;

  Future<void> initialize() async {
    if (_initialized) return;

    final appDir = await getApplicationDocumentsDirectory();
    _syncDirectory = Directory('${appDir.path}/OwlTransfer');

    if (!await _syncDirectory.exists()) {
      await _syncDirectory.create(recursive: true);
    }

    _initialized = true;
  }

  String get syncPath => _syncDirectory.path;

  Future<List<SyncFile>> listFiles({String? subPath}) async {
    await initialize();

    final targetDir = subPath != null
        ? Directory('${_syncDirectory.path}/$subPath')
        : _syncDirectory;

    if (!await targetDir.exists()) {
      return [];
    }

    final List<SyncFile> files = [];

    await for (final entity in targetDir.list(recursive: false)) {
      if (entity is File) {
        files.add(await SyncFile.fromFile(entity, _syncDirectory.path));
      } else if (entity is Directory) {
        files.add(await SyncFile.fromDirectory(entity, _syncDirectory.path));
      }
    }

    // Sort: directories first, then files alphabetically
    files.sort((a, b) {
      if (a.isDirectory && !b.isDirectory) return -1;
      if (!a.isDirectory && b.isDirectory) return 1;
      return a.name.toLowerCase().compareTo(b.name.toLowerCase());
    });

    return files;
  }

  Future<List<SyncFile>> listAllFilesRecursive() async {
    await initialize();

    final List<SyncFile> files = [];

    await for (final entity in _syncDirectory.list(recursive: true)) {
      if (entity is File) {
        files.add(await SyncFile.fromFile(entity, _syncDirectory.path));
      } else if (entity is Directory) {
        files.add(await SyncFile.fromDirectory(entity, _syncDirectory.path));
      }
    }

    return files;
  }

  Future<void> createFolder(String name, {String? parentPath}) async {
    await initialize();

    final path = parentPath != null
        ? '${_syncDirectory.path}/$parentPath/$name'
        : '${_syncDirectory.path}/$name';

    final dir = Directory(path);
    if (!await dir.exists()) {
      await dir.create(recursive: true);
    }
  }

  Future<void> addFile(File sourceFile, {String? targetPath}) async {
    await initialize();

    final fileName = sourceFile.path.split(Platform.pathSeparator).last;
    final destPath = targetPath != null
        ? '${_syncDirectory.path}/$targetPath/$fileName'
        : '${_syncDirectory.path}/$fileName';

    await sourceFile.copy(destPath);
  }

  Future<void> addFileFromBytes(String fileName, List<int> bytes, {String? targetPath}) async {
    await initialize();

    final destPath = targetPath != null
        ? '${_syncDirectory.path}/$targetPath/$fileName'
        : '${_syncDirectory.path}/$fileName';

    // Ensure parent directory exists
    final parent = Directory(destPath).parent;
    if (!await parent.exists()) {
      await parent.create(recursive: true);
    }

    final file = File(destPath);
    await file.writeAsBytes(bytes);
  }

  Future<List<int>> readFileBytes(String relativePath) async {
    await initialize();

    final file = File('${_syncDirectory.path}/$relativePath');
    return await file.readAsBytes();
  }

  Future<void> deleteFile(String relativePath) async {
    await initialize();

    final path = '${_syncDirectory.path}/$relativePath';
    final file = File(path);
    final dir = Directory(path);

    if (await file.exists()) {
      await file.delete();
    } else if (await dir.exists()) {
      await dir.delete(recursive: true);
    }
  }

  Future<void> renameFile(String oldPath, String newName) async {
    await initialize();

    final oldFullPath = '${_syncDirectory.path}/$oldPath';
    final entity = FileSystemEntity.typeSync(oldFullPath);

    final parentDir = oldFullPath.substring(0, oldFullPath.lastIndexOf(Platform.pathSeparator));
    final newFullPath = '$parentDir${Platform.pathSeparator}$newName';

    if (entity == FileSystemEntityType.file) {
      await File(oldFullPath).rename(newFullPath);
    } else if (entity == FileSystemEntityType.directory) {
      await Directory(oldFullPath).rename(newFullPath);
    }
  }

  Future<SyncManifest> generateManifest(String deviceId, String deviceName) async {
    final files = await listAllFilesRecursive();
    return SyncManifest(
      deviceId: deviceId,
      deviceName: deviceName,
      lastSync: DateTime.now(),
      files: files,
    );
  }
}
