import 'dart:io';
import 'package:flutter/material.dart';
import 'package:file_picker/file_picker.dart';
import '../models/sync_file.dart';
import '../services/file_service.dart';
import '../services/sync_service.dart';
import '../services/p2p_service.dart';
import '../services/settings_service.dart';
import 'pairing_screen.dart';
import 'settings_screen.dart';

class HomeScreen extends StatefulWidget {
  final FileService fileService;
  final SyncService syncService;
  final P2PService p2pService;
  final SettingsService settingsService;

  const HomeScreen({
    super.key,
    required this.fileService,
    required this.syncService,
    required this.p2pService,
    required this.settingsService,
  });

  @override
  State<HomeScreen> createState() => _HomeScreenState();
}

class _HomeScreenState extends State<HomeScreen> {
  List<SyncFile> _files = [];
  String? _currentPath;
  bool _isLoading = true;
  SyncStatus _syncStatus = SyncStatus.idle;

  @override
  void initState() {
    super.initState();
    _loadFiles();
    _setupListeners();
  }

  void _setupListeners() {
    widget.syncService.statusStream.listen((status) {
      setState(() => _syncStatus = status);
      if (status == SyncStatus.complete) {
        _loadFiles();
      }
    });
  }

  Future<void> _loadFiles() async {
    setState(() => _isLoading = true);

    final files = await widget.fileService.listFiles(subPath: _currentPath);

    setState(() {
      _files = files;
      _isLoading = false;
    });
  }

  void _navigateToFolder(String folderPath) {
    setState(() {
      _currentPath = _currentPath != null
          ? '$_currentPath/$folderPath'
          : folderPath;
    });
    _loadFiles();
  }

  void _navigateUp() {
    if (_currentPath == null) return;

    final parts = _currentPath!.split('/');
    if (parts.length <= 1) {
      setState(() => _currentPath = null);
    } else {
      setState(() => _currentPath = parts.sublist(0, parts.length - 1).join('/'));
    }
    _loadFiles();
  }

  Future<void> _createFolder() async {
    final controller = TextEditingController();

    final name = await showDialog<String>(
      context: context,
      builder: (context) => AlertDialog(
        title: const Text('New Folder'),
        content: TextField(
          controller: controller,
          decoration: const InputDecoration(
            hintText: 'Folder name',
          ),
          autofocus: true,
        ),
        actions: [
          TextButton(
            onPressed: () => Navigator.pop(context),
            child: const Text('Cancel'),
          ),
          TextButton(
            onPressed: () => Navigator.pop(context, controller.text),
            child: const Text('Create'),
          ),
        ],
      ),
    );

    if (name != null && name.isNotEmpty) {
      await widget.fileService.createFolder(name, parentPath: _currentPath);
      _loadFiles();
    }
  }

  Future<void> _addFile() async {
    final result = await FilePicker.platform.pickFiles(allowMultiple: true);

    if (result != null) {
      for (final file in result.files) {
        if (file.path != null) {
          await widget.fileService.addFile(
            File(file.path!),
            targetPath: _currentPath,
          );
        }
      }
      _loadFiles();
    }
  }

  Future<void> _deleteFile(SyncFile file) async {
    final confirm = await showDialog<bool>(
      context: context,
      builder: (context) => AlertDialog(
        title: const Text('Delete'),
        content: Text('Are you sure you want to delete "${file.name}"?'),
        actions: [
          TextButton(
            onPressed: () => Navigator.pop(context, false),
            child: const Text('Cancel'),
          ),
          TextButton(
            onPressed: () => Navigator.pop(context, true),
            style: TextButton.styleFrom(foregroundColor: Colors.red),
            child: const Text('Delete'),
          ),
        ],
      ),
    );

    if (confirm == true) {
      await widget.fileService.deleteFile(file.relativePath);
      _loadFiles();
    }
  }

  void _syncNow() {
    widget.syncService.syncWithAllDevices();
  }

  @override
  Widget build(BuildContext context) {
    return Scaffold(
      appBar: AppBar(
        title: Row(
          children: [
            const Text('🦉', style: TextStyle(fontSize: 24)),
            const SizedBox(width: 8),
            const Text('Owl Transfer'),
          ],
        ),
        leading: _currentPath != null
            ? IconButton(
                icon: const Icon(Icons.arrow_back),
                onPressed: _navigateUp,
              )
            : null,
        actions: [
          IconButton(
            icon: Icon(
              _syncStatus == SyncStatus.syncing
                  ? Icons.sync
                  : Icons.sync_disabled,
              color: _syncStatus == SyncStatus.syncing ? Colors.blue : null,
            ),
            onPressed: _syncNow,
            tooltip: 'Sync Now',
          ),
          IconButton(
            icon: const Icon(Icons.qr_code),
            onPressed: () => Navigator.push(
              context,
              MaterialPageRoute(
                builder: (context) => PairingScreen(
                  p2pService: widget.p2pService,
                  settingsService: widget.settingsService,
                ),
              ),
            ),
            tooltip: 'Pair Device',
          ),
          IconButton(
            icon: const Icon(Icons.settings),
            onPressed: () async {
              await Navigator.push(
                context,
                MaterialPageRoute(
                  builder: (context) => SettingsScreen(
                    settingsService: widget.settingsService,
                    syncService: widget.syncService,
                    p2pService: widget.p2pService,
                    fileService: widget.fileService,
                  ),
                ),
              );
              // Reload files in case sync folder changed
              _loadFiles();
            },
            tooltip: 'Settings',
          ),
        ],
      ),
      body: Column(
        children: [
          // Breadcrumb
          if (_currentPath != null)
            Container(
              padding: const EdgeInsets.symmetric(horizontal: 16, vertical: 8),
              color: Theme.of(context).colorScheme.surfaceContainerHighest,
              width: double.infinity,
              child: Text(
                '/$_currentPath',
                style: TextStyle(
                  color: Theme.of(context).colorScheme.onSurfaceVariant,
                ),
              ),
            ),

          // Sync status bar
          if (_syncStatus == SyncStatus.syncing)
            LinearProgressIndicator(
              backgroundColor: Colors.grey[200],
            ),

          // File list
          Expanded(
            child: _isLoading
                ? const Center(child: CircularProgressIndicator())
                : _files.isEmpty
                    ? Center(
                        child: Column(
                          mainAxisAlignment: MainAxisAlignment.center,
                          children: [
                            Icon(
                              Icons.folder_open,
                              size: 64,
                              color: Colors.grey[400],
                            ),
                            const SizedBox(height: 16),
                            Text(
                              'No files yet',
                              style: TextStyle(
                                color: Colors.grey[600],
                                fontSize: 18,
                              ),
                            ),
                            const SizedBox(height: 8),
                            Text(
                              'Tap + to add files or folders',
                              style: TextStyle(
                                color: Colors.grey[500],
                              ),
                            ),
                          ],
                        ),
                      )
                    : RefreshIndicator(
                        onRefresh: _loadFiles,
                        child: ListView.builder(
                          itemCount: _files.length,
                          itemBuilder: (context, index) {
                            final file = _files[index];
                            return _FileListTile(
                              file: file,
                              onTap: file.isDirectory
                                  ? () => _navigateToFolder(file.name)
                                  : null,
                              onDelete: () => _deleteFile(file),
                            );
                          },
                        ),
                      ),
          ),
        ],
      ),
      floatingActionButton: Column(
        mainAxisSize: MainAxisSize.min,
        children: [
          FloatingActionButton.small(
            heroTag: 'folder',
            onPressed: _createFolder,
            child: const Icon(Icons.create_new_folder),
          ),
          const SizedBox(height: 8),
          FloatingActionButton(
            heroTag: 'file',
            onPressed: _addFile,
            child: const Icon(Icons.add),
          ),
        ],
      ),
    );
  }
}

class _FileListTile extends StatelessWidget {
  final SyncFile file;
  final VoidCallback? onTap;
  final VoidCallback onDelete;

  const _FileListTile({
    required this.file,
    this.onTap,
    required this.onDelete,
  });

  String _formatSize(int bytes) {
    if (bytes < 1024) return '$bytes B';
    if (bytes < 1024 * 1024) return '${(bytes / 1024).toStringAsFixed(1)} KB';
    if (bytes < 1024 * 1024 * 1024) {
      return '${(bytes / (1024 * 1024)).toStringAsFixed(1)} MB';
    }
    return '${(bytes / (1024 * 1024 * 1024)).toStringAsFixed(1)} GB';
  }

  IconData _getFileIcon(String name) {
    final ext = name.split('.').last.toLowerCase();

    switch (ext) {
      case 'pdf':
        return Icons.picture_as_pdf;
      case 'jpg':
      case 'jpeg':
      case 'png':
      case 'gif':
      case 'webp':
        return Icons.image;
      case 'mp4':
      case 'mkv':
      case 'avi':
      case 'mov':
        return Icons.video_file;
      case 'mp3':
      case 'wav':
      case 'flac':
      case 'aac':
        return Icons.audio_file;
      case 'doc':
      case 'docx':
        return Icons.description;
      case 'xls':
      case 'xlsx':
        return Icons.table_chart;
      case 'zip':
      case 'rar':
      case '7z':
      case 'tar':
      case 'gz':
        return Icons.folder_zip;
      default:
        return Icons.insert_drive_file;
    }
  }

  @override
  Widget build(BuildContext context) {
    return ListTile(
      leading: Icon(
        file.isDirectory ? Icons.folder : _getFileIcon(file.name),
        color: file.isDirectory ? Colors.amber : Colors.blue,
        size: 40,
      ),
      title: Text(file.name),
      subtitle: file.isDirectory
          ? null
          : Text(_formatSize(file.size)),
      trailing: IconButton(
        icon: const Icon(Icons.delete_outline),
        onPressed: onDelete,
      ),
      onTap: onTap,
    );
  }
}
