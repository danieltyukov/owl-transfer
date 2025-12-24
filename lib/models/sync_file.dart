import 'dart:convert';
import 'dart:io';
import 'package:crypto/crypto.dart';

class SyncFile {
  final String id;
  final String name;
  final String relativePath;
  final bool isDirectory;
  final int size;
  final DateTime modifiedAt;
  final String? checksum;

  SyncFile({
    required this.id,
    required this.name,
    required this.relativePath,
    required this.isDirectory,
    required this.size,
    required this.modifiedAt,
    this.checksum,
  });

  Map<String, dynamic> toJson() => {
        'id': id,
        'name': name,
        'relativePath': relativePath,
        'isDirectory': isDirectory,
        'size': size,
        'modifiedAt': modifiedAt.toIso8601String(),
        'checksum': checksum,
      };

  factory SyncFile.fromJson(Map<String, dynamic> json) => SyncFile(
        id: json['id'],
        name: json['name'],
        relativePath: json['relativePath'],
        isDirectory: json['isDirectory'],
        size: json['size'],
        modifiedAt: DateTime.parse(json['modifiedAt']),
        checksum: json['checksum'],
      );

  static Future<SyncFile> fromFile(File file, String basePath) async {
    final stat = await file.stat();
    final relativePath = file.path.replaceFirst(basePath, '').replaceFirst(RegExp(r'^[/\\]'), '');
    final bytes = await file.readAsBytes();
    final checksum = md5.convert(bytes).toString();

    return SyncFile(
      id: '${relativePath}_${stat.modified.millisecondsSinceEpoch}',
      name: file.path.split(Platform.pathSeparator).last,
      relativePath: relativePath,
      isDirectory: false,
      size: stat.size,
      modifiedAt: stat.modified,
      checksum: checksum,
    );
  }

  static Future<SyncFile> fromDirectory(Directory dir, String basePath) async {
    final stat = await dir.stat();
    final relativePath = dir.path.replaceFirst(basePath, '').replaceFirst(RegExp(r'^[/\\]'), '');

    return SyncFile(
      id: '${relativePath}_dir',
      name: dir.path.split(Platform.pathSeparator).last,
      relativePath: relativePath,
      isDirectory: true,
      size: 0,
      modifiedAt: stat.modified,
    );
  }
}

class SyncManifest {
  final String deviceId;
  final String deviceName;
  final DateTime lastSync;
  final List<SyncFile> files;

  SyncManifest({
    required this.deviceId,
    required this.deviceName,
    required this.lastSync,
    required this.files,
  });

  Map<String, dynamic> toJson() => {
        'deviceId': deviceId,
        'deviceName': deviceName,
        'lastSync': lastSync.toIso8601String(),
        'files': files.map((f) => f.toJson()).toList(),
      };

  factory SyncManifest.fromJson(Map<String, dynamic> json) => SyncManifest(
        deviceId: json['deviceId'],
        deviceName: json['deviceName'],
        lastSync: DateTime.parse(json['lastSync']),
        files: (json['files'] as List).map((f) => SyncFile.fromJson(f)).toList(),
      );

  String toJsonString() => jsonEncode(toJson());

  factory SyncManifest.fromJsonString(String json) =>
      SyncManifest.fromJson(jsonDecode(json));
}
